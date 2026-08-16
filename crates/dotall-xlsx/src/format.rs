use std::path::{Path, PathBuf};

use dotall_core::registry::{
    ArtifactEnvelope, ArtifactSchema, Capability, DetectionProbe, DetectionScore, EditCapability,
    EncodedSnapshot, FormatDescriptor, FormatHandler, Inspection, PatchedOutput, ReadRequest,
    ReadResponse, ReadSelector, ReadSuggestion, SemanticOperation, ValidatedEdit,
};
use dotall_core::{DotallError, DotallStore, Result};
use serde_json::json;

use crate::detection;
use crate::edits;
use crate::model::{SCHEMA_ID, SCHEMA_VERSION, WorkbookModel};
use crate::{FORMAT_ID, parser, projection, selector, structure};

const AVAILABLE_READS: [&str; 6] = [
    "read.full",
    "read.sheet",
    "read.range",
    "read.ast_range",
    "read.named_ranges",
    "read.merges",
];
const PREVIEW_SHEET_LIMIT: usize = 3;
const PREVIEW_ROW_LIMIT: u32 = 10;
const PREVIEW_CELL_LIMIT: usize = 200;

pub struct XlsxFormat;

impl FormatHandler for XlsxFormat {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor {
            id: FORMAT_ID.into(),
            version: SCHEMA_VERSION.to_string(),
            capabilities: capabilities(),
            edit_capabilities: edit_capabilities(),
        }
    }

    fn artifact_schema(&self) -> ArtifactSchema {
        ArtifactSchema {
            format_id: FORMAT_ID.into(),
            schema_id: SCHEMA_ID.into(),
            schema_version: SCHEMA_VERSION,
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
                    "merges": sheet.merges,
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
                "named_ranges": workbook.named_ranges.iter().map(|range| json!({
                    "name": range.name,
                    "formula": range.formula,
                })).collect::<Vec<_>>(),
                "preserved": ["charts", "pivots", "vba", "other_ooxml_parts"],
                "structure": structure,
            }),
            capabilities: capabilities(),
            edit_capabilities: edit_capabilities(),
            suggested_reads,
        })
    }

    fn read(&self, model: &ArtifactEnvelope, request: &ReadRequest) -> Result<ReadResponse> {
        let workbook = decode(model)?;
        let selector =
            request
                .selector
                .as_ref()
                .map_or(Ok(selector::Selector::Preview), |selector| {
                    match selector.kind.as_str() {
                        "full" | "sheet" | "range" | "ast_range" | "named_ranges" | "merges" => {
                            selector::parse(&selector.kind, &selector.value).map_err(selector_error)
                        }
                        _ => Err(unsupported(&selector.kind)),
                    }
                })?;
        let (content, next_actions) = match selector {
            selector::Selector::Preview => render_preview(&workbook),
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
                let rendered = projection::markdown_sheet(sheet);
                (
                    rendered,
                    vec![format!(
                        "Use `ast_range` with `{}` for cells with element IDs",
                        used_range(sheet)
                    )],
                )
            }
            selector::Selector::Range { name, start, end } => {
                let sheet = find_sheet(&workbook, &name)?;
                let rendered = projection::markdown_range(sheet, start, end);
                let mut next_actions = vec![format!(
                    "Use `ast_range` with `{}!{}:{}` for cells with element IDs",
                    sheet.name,
                    address(start),
                    address(end)
                )];
                if rendered.truncated {
                    next_actions.push(format!(
                        "Range rendering was truncated; request a smaller range within `{}!{}:{}`",
                        sheet.name,
                        address(start),
                        address(end)
                    ));
                }
                (rendered.content, next_actions)
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
            selector::Selector::NamedRanges => (
                serde_json::to_string_pretty(&projection::named_ranges(&workbook)).map_err(
                    |source| DotallError::Serialization {
                        context: "XLSX named ranges projection".into(),
                        source,
                    },
                )?,
                vec![
                    "Use `merges` with a sheet name for worksheet merge refs".into(),
                    "Use `range` or `ast_range` for cell values".into(),
                ],
            ),
            selector::Selector::Merges { name } => {
                let sheet = find_sheet(&workbook, &name)?;
                (
                    serde_json::to_string_pretty(&projection::merges(sheet)).map_err(|source| {
                        DotallError::Serialization {
                            context: "XLSX merges projection".into(),
                            source,
                        }
                    })?,
                    vec![
                        "Use `named_ranges` for defined name formulas".into(),
                        format!(
                            "Use `range` with `{}` for a Markdown table",
                            used_range(sheet)
                        ),
                    ],
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

    fn query_dependencies(
        &self,
        store: &DotallStore,
        relative: &str,
        model: &ArtifactEnvelope,
        source_hash: &str,
        selector: &str,
        dependents: bool,
    ) -> Result<serde_json::Value> {
        let workbook = decode(model)?;
        let element_id = resolve_cell_element_id(&workbook, selector)?;
        let direction = if dependents {
            crate::dependencies::DependencyDirection::Reverse
        } else {
            crate::dependencies::DependencyDirection::Forward
        };
        let result = crate::dependencies::ensure_and_query(
            store,
            relative,
            model,
            source_hash,
            &element_id,
            direction,
        )?;
        serde_json::to_value(result).map_err(|source| DotallError::Serialization {
            context: "XLSX dependency query".into(),
            source,
        })
    }

    fn validate_edit(
        &self,
        model: &ArtifactEnvelope,
        operations: &[SemanticOperation],
    ) -> Result<ValidatedEdit> {
        edits::validate(model, operations)
    }

    fn validate_edit_with_source(
        &self,
        source: &Path,
        model: &ArtifactEnvelope,
        operations: &[SemanticOperation],
    ) -> Result<ValidatedEdit> {
        edits::validate_with_source(source, model, operations)
    }

    fn apply_edit(&self, source: &Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
        edits::writer::apply(source, edit)
    }

    fn encode_snapshot(&self, source_bytes: &[u8]) -> Result<EncodedSnapshot> {
        crate::snapshot::encode(source_bytes)
    }

    fn decode_snapshot(&self, encoded: &EncodedSnapshot) -> Result<Vec<u8>> {
        crate::snapshot::decode(encoded)
    }
}

fn render_preview(workbook: &WorkbookModel) -> (String, Vec<String>) {
    let mut content = String::from("# Workbook summary\n\n");
    for sheet in &workbook.sheets {
        content.push_str(&format!(
            "- `{}`: {} rows × {} columns\n",
            sheet.name, sheet.dimensions.rows, sheet.dimensions.cols
        ));
    }

    let mut next_actions = Vec::new();
    for sheet in workbook.sheets.iter().take(PREVIEW_SHEET_LIMIT) {
        let rendered = projection::markdown_range_with_limit(
            sheet,
            selector::CellAddress { row: 1, col: 1 },
            selector::CellAddress {
                row: sheet.dimensions.rows.clamp(1, PREVIEW_ROW_LIMIT),
                col: sheet.dimensions.cols.max(1),
            },
            PREVIEW_CELL_LIMIT,
        );
        content.push('\n');
        content.push_str(&rendered.content);
        next_actions.push(format!("Read `{}` with selector kind `sheet`", sheet.name));
        if rendered.truncated {
            next_actions.push(format!(
                "Preview of `{}` is capped; request a smaller `range` for more cells",
                sheet.name
            ));
        }
    }
    if workbook.sheets.len() > PREVIEW_SHEET_LIMIT {
        next_actions.push(format!(
            "Preview shows the first {PREVIEW_SHEET_LIMIT} sheets; read another sheet by name"
        ));
    }

    (content, next_actions)
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
        Capability::ReadSelector {
            kind: "named_ranges".into(),
        },
        Capability::ReadSelector {
            kind: "merges".into(),
        },
    ]
}

fn edit_capabilities() -> Vec<EditCapability> {
    vec![
        EditCapability {
            operation: "set_cell_value".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set a cell to a string, number, boolean, or blank value.".into(),
            example: json!({
                "kind": "set_cell_value",
                "payload": { "sheet": "Sheet1", "address": "A1", "value": 42 }
            }),
            safety: "Surgically patches the target worksheet and preserves unrelated OOXML parts."
                .into(),
        },
        EditCapability {
            operation: "set_cell_formula".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set a cell formula without evaluating it.".into(),
            example: json!({
                "kind": "set_cell_formula",
                "payload": { "sheet": "Sheet1", "address": "B1", "formula": "=A1*2" }
            }),
            safety: "Surgically patches the target worksheet and preserves unrelated OOXML parts."
                .into(),
        },
        EditCapability {
            operation: "set_range".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description:
                "Set a rectangular range of values and formulas without evaluating formulas.".into(),
            example: json!({
                "kind": "set_range",
                "payload": {
                    "sheet": "Sheet1",
                    "start_cell": "A1",
                    "values": [
                        [1, { "kind": "formula", "value": "=A1*2" }]
                    ]
                }
            }),
            safety: "Validates the complete rectangle before one surgical worksheet patch pass."
                .into(),
        },
        EditCapability {
            operation: "insert_row".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Insert blank worksheet rows and rewrite affected formulas.".into(),
            example: json!({ "kind": "insert_row", "payload": { "sheet": "Sheet1", "at": 2, "count": 1 } }),
            safety: "Rejects unsupported structural impacts before surgically patching worksheets."
                .into(),
        },
        EditCapability {
            operation: "delete_row".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Delete worksheet rows and rewrite affected formulas.".into(),
            example: json!({ "kind": "delete_row", "payload": { "sheet": "Sheet1", "at": 2, "count": 1 } }),
            safety: "Rejects unsupported structural impacts before surgically patching worksheets."
                .into(),
        },
        EditCapability {
            operation: "insert_column".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Insert blank worksheet columns and rewrite affected formulas.".into(),
            example: json!({ "kind": "insert_column", "payload": { "sheet": "Sheet1", "at": 2, "count": 1 } }),
            safety: "Rejects unsupported structural impacts before surgically patching worksheets."
                .into(),
        },
        EditCapability {
            operation: "delete_column".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Delete worksheet columns and rewrite affected formulas.".into(),
            example: json!({ "kind": "delete_column", "payload": { "sheet": "Sheet1", "at": 2, "count": 1 } }),
            safety: "Rejects unsupported structural impacts before surgically patching worksheets."
                .into(),
        },
        EditCapability {
            operation: "add_sheet".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Add a blank worksheet, optionally after an existing sheet.".into(),
            example: json!({ "kind": "add_sheet", "payload": { "name": "Data", "after": "Sheet1" } }),
            safety: "Surgically patches workbook metadata and adds only the new worksheet part."
                .into(),
        },
        EditCapability {
            operation: "rename_sheet".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Rename a worksheet and rewrite supported sheet-qualified formulas.".into(),
            example: json!({ "kind": "rename_sheet", "payload": { "from": "Sheet1", "to": "Data" } }),
            safety: "Rejects charts, tables, defined names, and other unsupported parts that reference the renamed sheet."
                .into(),
        },
        EditCapability {
            operation: "delete_sheet".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Delete a worksheet after checking inbound formula and defined-name references.".into(),
            example: json!({
                "kind": "delete_sheet",
                "payload": {
                    "name": "Sheet1",
                    "dependency_policy": "reject_if_referenced"
                }
            }),
            safety: "Rejects the last visible sheet; either lists inbound references or rewrites them to #REF! while removing only orphaned drawings and charts."
                .into(),
        },
        EditCapability {
            operation: "merge_cells".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Merge a rectangular worksheet range.".into(),
            example: json!({
                "kind": "merge_cells",
                "payload": { "sheet": "Sheet1", "range": "A1:B2" }
            }),
            safety: "Rejects overlapping merges; surgically patches only the target worksheet mergeCells."
                .into(),
        },
        EditCapability {
            operation: "unmerge_cells".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Remove an existing worksheet merge by exact range ref.".into(),
            example: json!({
                "kind": "unmerge_cells",
                "payload": { "sheet": "Sheet1", "range": "A1:B2" }
            }),
            safety: "Removes the matching mergeCell ref; surgically patches only the target worksheet."
                .into(),
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

fn resolve_cell_element_id(workbook: &WorkbookModel, selector: &str) -> Result<String> {
    let (sheet_name, address) =
        selector
            .rsplit_once('!')
            .ok_or_else(|| DotallError::UnsupportedCapability {
                format_id: FORMAT_ID.into(),
                capability: "invalid cell selector".into(),
                available: vec!["use Sheet!A1".into()],
            })?;
    let address = address.to_ascii_uppercase();

    workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(sheet_name))
        .and_then(|sheet| {
            sheet
                .cells
                .iter()
                .find(|cell| cell.address.eq_ignore_ascii_case(&address))
        })
        .map(|cell| cell.element_id.clone())
        .ok_or_else(|| DotallError::UnsupportedCapability {
            format_id: FORMAT_ID.into(),
            capability: format!("unknown cell selector {selector}"),
            available: vec!["use an existing Sheet!A1 cell address".into()],
        })
}

fn find_sheet<'a>(workbook: &'a WorkbookModel, name: &str) -> Result<&'a crate::model::SheetModel> {
    workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == name)
        .ok_or_else(|| selector_error(format!("sheet not found: {name}")))
}

fn selector_error(message: String) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: PathBuf::from("<read selector>"),
        message,
    }
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
