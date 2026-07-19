use std::collections::{BTreeMap, BTreeSet};

use dotall_core::ArtifactEnvelope;
use serde::{Deserialize, Serialize};

use crate::dependencies::{CellReference, FormulaToken, lex};
use crate::{FORMAT_ID, WorkbookModel};

pub const SCHEMA_ID: &str = "xlsx.formula-dependencies";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DependencyTarget {
    Element { element_id: String },
    Selector { selector: String },
    NamedRange { element_id: String, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub from_element_id: String,
    pub to: DependencyTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub edges: Vec<DependencyEdge>,
    #[serde(default)]
    cell_locations: BTreeMap<String, CellLocation>,
}

pub fn build(model: &WorkbookModel) -> DependencyGraph {
    let cells = cell_index(model);
    let named_ranges = model
        .named_ranges
        .iter()
        .map(|named_range| (named_range.name.to_ascii_uppercase(), named_range))
        .collect::<BTreeMap<_, _>>();
    let mut edges = BTreeSet::new();

    for sheet in &model.sheets {
        for cell in &sheet.cells {
            let Some(formula) = &cell.formula else {
                continue;
            };

            for token in lex(formula) {
                match token {
                    FormulaToken::Reference(reference) => {
                        edges.insert(DependencyEdge {
                            from_element_id: cell.element_id.clone(),
                            to: reference_target(&reference, &sheet.name, &cells),
                        });
                    }
                    FormulaToken::NamedRange { name } => {
                        let Some(named_range) = named_ranges.get(&name.to_ascii_uppercase()) else {
                            continue;
                        };
                        let resolved = named_range_targets(named_range.formula.as_str(), &cells);
                        if resolved.is_empty() {
                            edges.insert(DependencyEdge {
                                from_element_id: cell.element_id.clone(),
                                to: DependencyTarget::NamedRange {
                                    element_id: named_range.element_id.clone(),
                                    name: named_range.name.clone(),
                                },
                            });
                        } else {
                            for to in resolved {
                                edges.insert(DependencyEdge {
                                    from_element_id: cell.element_id.clone(),
                                    to,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    DependencyGraph {
        edges: edges.into_iter().collect(),
        cell_locations: model
            .sheets
            .iter()
            .flat_map(|sheet| {
                sheet.cells.iter().map(move |cell| {
                    (
                        cell.element_id.clone(),
                        CellLocation {
                            sheet: sheet.name.clone(),
                            address: cell.address.clone(),
                        },
                    )
                })
            })
            .collect(),
    }
}

pub fn to_artifact(graph: &DependencyGraph) -> ArtifactEnvelope {
    ArtifactEnvelope {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        payload: serde_json::json!({
            "edges": graph.edges,
            "cell_locations": graph.cell_locations,
        }),
    }
}

impl DependencyGraph {
    pub fn forward(&self, element_id: &str) -> Vec<&DependencyEdge> {
        self.edges
            .iter()
            .filter(|edge| edge.from_element_id == element_id)
            .collect()
    }

    pub fn reverse(&self, element_id: &str) -> Vec<&DependencyEdge> {
        let Some(source) = self.cell_locations.get(element_id) else {
            return Vec::new();
        };

        self.edges
            .iter()
            .filter(|edge| match &edge.to {
                DependencyTarget::Element { element_id: target } => target == element_id,
                DependencyTarget::Selector { selector } => selector_contains(selector, source),
                DependencyTarget::NamedRange { .. } => false,
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CellLocation {
    sheet: String,
    address: String,
}

type CellIndex = BTreeMap<(String, String), String>;

fn cell_index(model: &WorkbookModel) -> CellIndex {
    model
        .sheets
        .iter()
        .flat_map(|sheet| {
            let sheet_key = sheet.name.to_ascii_uppercase();
            sheet.cells.iter().map(move |cell| {
                (
                    (sheet_key.clone(), cell.address.to_ascii_uppercase()),
                    cell.element_id.clone(),
                )
            })
        })
        .collect()
}

fn named_range_targets(formula: &str, cells: &CellIndex) -> Vec<DependencyTarget> {
    lex(formula)
        .into_iter()
        .filter_map(|token| match token {
            FormulaToken::Reference(reference) => {
                let sheet = reference.sheet.as_deref()?;
                Some(reference_target(&reference, sheet, cells))
            }
            FormulaToken::NamedRange { .. } => None,
        })
        .collect()
}

fn reference_target(
    reference: &crate::dependencies::FormulaReference,
    current_sheet: &str,
    cells: &CellIndex,
) -> DependencyTarget {
    let sheet = reference
        .sheet
        .as_deref()
        .unwrap_or(current_sheet);
    let sheet_key = sheet.to_ascii_uppercase();
    let start = address(&reference.start);

    match &reference.end {
        Some(end) => DependencyTarget::Selector {
            selector: format!("{sheet}!{start}:{}", address(end)),
        },
        None => cells
            .get(&(sheet_key, start.clone()))
            .map(|element_id| DependencyTarget::Element {
                element_id: element_id.clone(),
            })
            .unwrap_or_else(|| DependencyTarget::Selector {
                selector: format!("{sheet}!{start}"),
            }),
    }
}

fn address(reference: &CellReference) -> String {
    format!("{}{}", reference.column, reference.row)
}

fn selector_contains(selector: &str, source: &CellLocation) -> bool {
    let Some((sheet, start, end)) = parse_selector(selector) else {
        return false;
    };
    if !sheet.eq_ignore_ascii_case(&source.sheet) {
        return false;
    }

    let Some(source_coordinates) = coordinates(&source.address) else {
        return false;
    };
    let Some(start_coordinates) = coordinates(&start) else {
        return false;
    };
    let Some(end_coordinates) = coordinates(end.as_deref().unwrap_or(&start)) else {
        return false;
    };

    let (min_column, max_column) = ordered(start_coordinates.0, end_coordinates.0);
    let (min_row, max_row) = ordered(start_coordinates.1, end_coordinates.1);
    (min_column..=max_column).contains(&source_coordinates.0)
        && (min_row..=max_row).contains(&source_coordinates.1)
}

fn parse_selector(selector: &str) -> Option<(String, String, Option<String>)> {
    let (sheet, reference) = selector.rsplit_once('!')?;
    let (start, end) = reference
        .split_once(':')
        .map_or((reference, None), |(start, end)| (start, Some(end)));
    Some((
        sheet.to_owned(),
        start.to_ascii_uppercase(),
        end.map(str::to_ascii_uppercase),
    ))
}

fn coordinates(address: &str) -> Option<(u32, u32)> {
    let split = address.find(|character: char| character.is_ascii_digit())?;
    let (letters, digits) = address.split_at(split);
    let column = letters.bytes().try_fold(0_u32, |total, letter| {
        total
            .checked_mul(26)?
            .checked_add(u32::from(letter.to_ascii_uppercase() - b'A' + 1))
    })?;
    Some((column, digits.parse().ok()?))
}

fn ordered(first: u32, second: u32) -> (u32, u32) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}
