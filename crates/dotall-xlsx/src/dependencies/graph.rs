use std::collections::{BTreeMap, BTreeSet};

use dotall_core::{
    ArtifactEnvelope, CachedDerived, DerivationRecipe, DotallError, DotallStore, Result,
};
use serde::{Deserialize, Serialize};

use crate::dependencies::{CellReference, FormulaToken, lex};
use crate::model::{SCHEMA_ID as MODEL_SCHEMA_ID, SCHEMA_VERSION as MODEL_SCHEMA_VERSION};
use crate::{FORMAT_ID, WorkbookModel};

pub const SCHEMA_ID: &str = "xlsx.formula-dependencies";
pub const SCHEMA_VERSION: u32 = 1;
const PROCESSOR_ID: &str = "xlsx.formula-dependencies";
const PROCESSOR_VERSION: &str = "1";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyDirection {
    Forward,
    Reverse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepsQueryResult {
    pub cache_hit: bool,
    pub direction: DependencyDirection,
    pub edges: Vec<DependencyEdge>,
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

pub fn ensure_formula_dependencies(
    store: &DotallStore,
    relative: &str,
    model: &ArtifactEnvelope,
    source_hash: &str,
) -> Result<(DependencyGraph, bool)> {
    let workbook = decode_workbook(model)?;
    let recipe = recipe(model, source_hash)?;

    if let Some(cached) =
        store.read_derived_for_recipe(relative, &recipe, MODEL_SCHEMA_ID, MODEL_SCHEMA_VERSION)?
    {
        return Ok((decode_graph(cached.payload)?, true));
    }

    let graph = build(&workbook);
    store.write_derived_for_recipe(
        relative,
        &recipe,
        &CachedDerived {
            source_hash: source_hash.into(),
            model_schema_id: MODEL_SCHEMA_ID.into(),
            model_schema_version: MODEL_SCHEMA_VERSION,
            processor_id: PROCESSOR_ID.into(),
            processor_version: PROCESSOR_VERSION.into(),
            payload: serde_json::to_value(&graph).map_err(|source| DotallError::Serialization {
                context: "formula dependency graph".into(),
                source,
            })?,
        },
    )?;

    Ok((graph, false))
}

pub fn ensure_and_query(
    store: &DotallStore,
    relative: &str,
    model: &ArtifactEnvelope,
    source_hash: &str,
    cell_selector: &str,
    direction: DependencyDirection,
) -> Result<DepsQueryResult> {
    let (graph, cache_hit) = ensure_formula_dependencies(store, relative, model, source_hash)?;
    let edges = match direction {
        DependencyDirection::Forward => graph.forward(cell_selector),
        DependencyDirection::Reverse => graph.reverse(cell_selector),
    }
    .into_iter()
    .cloned()
    .collect();

    Ok(DepsQueryResult {
        cache_hit,
        direction,
        edges,
    })
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

        self.reverse_at(&source.sheet, &source.address)
    }

    pub fn reverse_at(&self, sheet: &str, address: &str) -> Vec<&DependencyEdge> {
        let source = CellLocation {
            sheet: sheet.to_owned(),
            address: address.to_ascii_uppercase(),
        };

        self.edges
            .iter()
            .filter(|edge| match &edge.to {
                DependencyTarget::Element { element_id: target } => self
                    .cell_locations
                    .get(target)
                    .is_some_and(|location| location_matches(location, &source)),
                DependencyTarget::Selector { selector } => selector_contains(selector, &source),
                DependencyTarget::NamedRange { .. } => false,
            })
            .collect()
    }
}

fn recipe(model: &ArtifactEnvelope, source_hash: &str) -> Result<DerivationRecipe> {
    let model_bytes =
        serde_json::to_vec(&model.payload).map_err(|source| DotallError::Serialization {
            context: "XLSX workbook model cache input".into(),
            source,
        })?;

    Ok(DerivationRecipe {
        source_hash: source_hash.into(),
        processor_id: PROCESSOR_ID.into(),
        processor_version: PROCESSOR_VERSION.into(),
        config_hash: blake3::hash(b"{}").to_hex().to_string(),
        input_hashes: vec![blake3::hash(&model_bytes).to_hex().to_string()],
    })
}

fn decode_workbook(model: &ArtifactEnvelope) -> Result<WorkbookModel> {
    if model.format_id != FORMAT_ID
        || model.schema_id != MODEL_SCHEMA_ID
        || model.schema_version != MODEL_SCHEMA_VERSION
    {
        return Err(DotallError::ArtifactSchemaMismatch {
            format_id: FORMAT_ID.into(),
            schema_id: model.schema_id.clone(),
            schema_version: model.schema_version,
        });
    }

    serde_json::from_value(model.payload.clone()).map_err(|source| DotallError::Serialization {
        context: "XLSX workbook artifact payload".into(),
        source,
    })
}

fn decode_graph(payload: serde_json::Value) -> Result<DependencyGraph> {
    serde_json::from_value(payload).map_err(|source| DotallError::Serialization {
        context: "cached formula dependency graph".into(),
        source,
    })
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
    let sheet = reference.sheet.as_deref().unwrap_or(current_sheet);
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

fn location_matches(location: &CellLocation, source: &CellLocation) -> bool {
    location.sheet.eq_ignore_ascii_case(&source.sheet)
        && location.address.eq_ignore_ascii_case(&source.address)
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
