use std::path::Path;

use dotall_core::registry::{
    ArtifactEnvelope, Capability, DetectionProbe, DetectionScore, FormatDescriptor, FormatHandler,
    Inspection, ReadRequest, ReadResponse, ReadSelector, ReadSuggestion,
};
use dotall_core::{DotallError, Result};
use serde_json::json;

use crate::detection;
use crate::model::{SCHEMA_ID, SCHEMA_VERSION, WorkbookModel};
use crate::{FORMAT_ID, parser, projection, selector, structure};

const AVAILABLE_READS: [&str; 4] = ["read.full", "read.sheet", "read.range", "read.ast_range"];

pub struct XlsxFormat;

impl FormatHandler for XlsxFormat {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor {
            id: FORMAT_ID.into(),
            version: SCHEMA_VERSION.to_string(),
            capabilities: capabilities(),
        }
    }

    fn detect(&self, probe: &DetectionProbe<'_>) -> DetectionScore {
        detection::score(probe)
    }

    fn parse(&self, source: &Path) -> Result<ArtifactEnvelope> {
        let payload = serde_json::to_value(parser::parse_workbook(source)?).map_err(|source| {
            DotallError::Serialization {
                context: "XLSX workbook model".into(),
                source,
            }
        })?;

        Ok(ArtifactEnvelope {
            format_id: FORMAT_ID.into(),
            schema_id: SCHEMA_ID.into(),
            schema_version: SCHEMA_VERSION,
            payload,
        })
    }

    fn inspect(&self, model: &ArtifactEnvelope) -> Result<Inspection> {
        let workbook = decode(model)?;
        let structure = structure::analyze(&workbook);
        let sheets = workbook
            .sheets
            .iter()
            .map(|sheet| {
                json!({
                    "name": sheet.name,
                    "rows": sheet.dimensions.rows,
                    "cols": sheet.dimensions.cols,
                    "formula_count": sheet.cells.iter().filter(|cell| cell.formula.is_some()).count(),
                })
            })
            .collect::<Vec<_>>();
        let suggested_reads = workbook
            .sheets
            .iter()
            .flat_map(|sheet| {
                let range = used_range(sheet);
                [
                    ReadSuggestion {
                        description: format!("{} table preview", sheet.name),
                        selector: ReadSelector {
                            kind: "range".into(),
                            value: range.clone(),
                        },
                    },
                    ReadSuggestion {
                        description: format!("Exact {} cells as JSON", sheet.name),
                        selector: ReadSelector {
                            kind: "ast_range".into(),
                            value: range,
                        },
                    },
                ]
            })
            .collect();

        Ok(Inspection {
            format_id: FORMAT_ID.into(),
            summary: json!({
                "sheets": sheets,
                "named_ranges": workbook.named_ranges.iter().map(|range| &range.name).collect::<Vec<_>>(),
                "preserved": ["charts", "pivots", "vba", "other_ooxml_parts"],
                "structure": structure,
            }),
            capabilities: capabilities(),
            suggested_reads,
        })
    }

    fn read(&self, model: &ArtifactEnvelope, request: &ReadRequest) -> Result<ReadResponse> {
        let workbook = decode(model)?;
        let selector =
            request
                .selector
                .as_ref()
                .map_or(Ok(selector::Selector::Full), |selector| {
                    selector::parse(&selector.kind, &selector.value)
                        .map_err(|_| unsupported(&selector.kind))
                })?;
        let (content, next_actions) = match selector {
            selector::Selector::Full => (
                projection::markdown_workbook(&workbook),
                workbook
                    .sheets
                    .iter()
                    .map(|sheet| format!("Read `{}` with selector kind `sheet`", sheet.name))
                    .collect(),
            ),
            selector::Selector::Sheet { name } => {
                let sheet = find_sheet(&workbook, &name)?;
                (
                    projection::markdown_sheet(sheet),
                    vec![format!(
                        "Use `ast_range` with `{}` for cells with element IDs",
                        used_range(sheet)
                    )],
                )
            }
            selector::Selector::Range { name, start, end } => {
                let sheet = find_sheet(&workbook, &name)?;
                (
                    projection::markdown_range(sheet, start, end),
                    vec![format!(
                        "Use `ast_range` with `{}!{}:{}` for cells with element IDs",
                        sheet.name,
                        address(start),
                        address(end)
                    )],
                )
            }
            selector::Selector::AstRange { name, start, end } => {
                let sheet = find_sheet(&workbook, &name)?;
                (
                    serde_json::to_string_pretty(&projection::ast_range(sheet, start, end))
                        .map_err(|source| DotallError::Serialization {
                            context: "XLSX AST range projection".into(),
                            source,
                        })?,
                    vec![format!(
                        "Use `range` with `{}!{}:{}` for a Markdown table",
                        sheet.name,
                        address(start),
                        address(end)
                    )],
                )
            }
        };
        let estimated_tokens = content.split_whitespace().count();

        Ok(ReadResponse {
            content,
            estimated_tokens,
            truncated: false,
            continuation: None,
            next_actions,
        })
    }
}

fn capabilities() -> Vec<Capability> {
    vec![
        Capability::Inspect,
        Capability::ReadFull,
        Capability::ReadSelector {
            kind: "sheet".into(),
        },
        Capability::ReadSelector {
            kind: "range".into(),
        },
        Capability::ReadSelector {
            kind: "ast_range".into(),
        },
    ]
}

fn decode(model: &ArtifactEnvelope) -> Result<WorkbookModel> {
    if model.format_id != FORMAT_ID
        || model.schema_id != SCHEMA_ID
        || model.schema_version != SCHEMA_VERSION
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

fn find_sheet<'a>(workbook: &'a WorkbookModel, name: &str) -> Result<&'a crate::model::SheetModel> {
    workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == name)
        .ok_or_else(|| unsupported("read.sheet"))
}

fn unsupported(capability: &str) -> DotallError {
    DotallError::UnsupportedCapability {
        format_id: FORMAT_ID.into(),
        capability: capability.into(),
        available: AVAILABLE_READS.iter().map(ToString::to_string).collect(),
    }
}

fn used_range(sheet: &crate::model::SheetModel) -> String {
    format!(
        "{}!A1:{}{}",
        sheet.name,
        column_name(sheet.dimensions.cols.max(1)),
        sheet.dimensions.rows.max(1)
    )
}

fn address(cell: selector::CellAddress) -> String {
    format!("{}{}", column_name(cell.col), cell.row)
}

fn column_name(mut col: u32) -> String {
    let mut letters = Vec::new();
    while col > 0 {
        col -= 1;
        letters.push((b'A' + (col % 26) as u8) as char);
        col /= 26;
    }
    letters.iter().rev().collect()
}
