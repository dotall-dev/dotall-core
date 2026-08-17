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
                    "freeze_panes": sheet.freeze_panes,
                    "zoom": sheet.zoom,
                    "show_gridlines": sheet.show_gridlines,
                    "tab_color": sheet.tab_color,
                    "auto_filter": sheet.auto_filter,
                    "print_area": sheet.print_area,
                    "print_titles": sheet.print_titles,
                    "page_orientation": sheet.page_orientation,
                    "paper_size": sheet.paper_size,
                    "print_scale": sheet.print_scale,
                    "fit_to_page": sheet.fit_to_page,
                    "center_on_page": sheet.center_on_page,
                    "page_margins": sheet.page_margins,
                    "header_footer": sheet.header_footer,
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
                "style_table": workbook.style_table.iter().map(|entry| json!({
                    "style_id": entry.style_id,
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
        EditCapability {
            operation: "set_column_width".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set a worksheet column width (character units).".into(),
            example: json!({
                "kind": "set_column_width",
                "payload": { "sheet": "Sheet1", "column": "A", "width": 18.5 }
            }),
            safety: "Surgically patches only the target worksheet cols; other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_row_height".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set a worksheet row height (points).".into(),
            example: json!({
                "kind": "set_row_height",
                "payload": { "sheet": "Sheet1", "row": 1, "height": 30.0 }
            }),
            safety: "Surgically patches only the target worksheet row attributes; other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "freeze_panes".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Freeze worksheet panes at a cell (openpyxl-style), or clear with null/A1."
                .into(),
            example: json!({
                "kind": "freeze_panes",
                "payload": { "sheet": "Sheet1", "cell": "B2" }
            }),
            safety: "Surgically patches only the target worksheet sheetViews; other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "define_name".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Create or update a workbook-scoped defined name formula.".into(),
            example: json!({
                "kind": "define_name",
                "payload": { "name": "Rate", "formula": "Inputs!$B$3" }
            }),
            safety: "Surgically patches only xl/workbook.xml definedNames; worksheets and other parts stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "delete_name".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Remove a workbook-scoped defined name from xl/workbook.xml."
                .into(),
            example: json!({
                "kind": "delete_name",
                "payload": { "name": "Rate" }
            }),
            safety: "Surgically patches only xl/workbook.xml definedNames; worksheets and other parts stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "hide_sheet".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Hide or unhide a worksheet via workbook sheet state=\"hidden\"."
                .into(),
            example: json!({
                "kind": "hide_sheet",
                "payload": { "sheet": "Revenue", "hidden": true }
            }),
            safety: "Surgically patches only xl/workbook.xml sheet state. Rejects hiding the last visible sheet. Worksheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_tab_color".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set or clear a worksheet tab color (sheetPr/tabColor rgb)."
                .into(),
            example: json!({
                "kind": "set_tab_color",
                "payload": { "sheet": "Inputs", "color": "FF4472C4" }
            }),
            safety: "Surgically patches only the target worksheet sheetPr/tabColor. Pass null color to clear. Other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_auto_filter".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set or clear a worksheet AutoFilter range.".into(),
            example: json!({
                "kind": "set_auto_filter",
                "payload": { "sheet": "Inputs", "range": "A1:B10" }
            }),
            safety: "Surgically patches only the target worksheet autoFilter element. Pass null range to clear. Other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_print_area".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set or clear a worksheet print area via workbook _xlnm.Print_Area."
                .into(),
            example: json!({
                "kind": "set_print_area",
                "payload": { "sheet": "Revenue", "range": "A1:B5" }
            }),
            safety: "Surgically patches only xl/workbook.xml definedNames for _xlnm.Print_Area. Pass null range to clear. Worksheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_print_titles".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description:
                "Set or clear worksheet print titles (repeat rows/cols) via _xlnm.Print_Titles."
                    .into(),
            example: json!({
                "kind": "set_print_titles",
                "payload": { "sheet": "Revenue", "rows": "1:1", "cols": "A:A" }
            }),
            safety: "Surgically patches only xl/workbook.xml definedNames for _xlnm.Print_Titles. Pass null rows and cols to clear. Worksheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_page_orientation".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set worksheet print orientation via pageSetup (portrait or landscape)."
                .into(),
            example: json!({
                "kind": "set_page_orientation",
                "payload": { "sheet": "Revenue", "orientation": "landscape" }
            }),
            safety: "Surgically patches only the target worksheet pageSetup orientation attribute. Other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_paper_size".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set worksheet print paper size via pageSetup paperSize (positive integer)."
                .into(),
            example: json!({
                "kind": "set_paper_size",
                "payload": { "sheet": "Revenue", "paper_size": 9 }
            }),
            safety: "Surgically patches only the target worksheet pageSetup paperSize attribute (e.g. 1=Letter, 9=A4). Inspect surfaces paper_size. Other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_print_scale".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set worksheet print scale via pageSetup (percent 10–400).".into(),
            example: json!({
                "kind": "set_print_scale",
                "payload": { "sheet": "Revenue", "scale": 75 }
            }),
            safety: "Surgically patches only the target worksheet pageSetup scale attribute. Inspect surfaces print_scale. Other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_fit_to_page".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description:
                "Set or clear worksheet fit-to-page via pageSetup fitToWidth/Height and pageSetUpPr."
                    .into(),
            example: json!({
                "kind": "set_fit_to_page",
                "payload": { "sheet": "Revenue", "width": 1, "height": 1 }
            }),
            safety: "Surgically patches only the target worksheet sheetPr/pageSetUpPr and pageSetup fit attrs. Pass null width and height to clear. Inspect surfaces fit_to_page. Other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_center_on_page".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description:
                "Set or clear worksheet print centering via printOptions horizontalCentered/verticalCentered."
                    .into(),
            example: json!({
                "kind": "set_center_on_page",
                "payload": { "sheet": "Revenue", "horizontal": true, "vertical": false }
            }),
            safety: "Surgically patches only the target worksheet printOptions. Both false clears centering. Inspect surfaces center_on_page when either axis is centered. Other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_page_margins".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set worksheet print margins via pageMargins (inches)."
                .into(),
            example: json!({
                "kind": "set_page_margins",
                "payload": {
                    "sheet": "Revenue",
                    "left": 0.5,
                    "right": 0.5,
                    "top": 0.75,
                    "bottom": 0.75,
                    "header": 0.3,
                    "footer": 0.3
                }
            }),
            safety: "Surgically patches only the target worksheet pageMargins element. Requires left/right/top/bottom; header/footer optional. Other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_header_footer".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description:
                "Set or clear worksheet print header/footer via headerFooter oddHeader/oddFooter."
                    .into(),
            example: json!({
                "kind": "set_header_footer",
                "payload": { "sheet": "Revenue", "header": "&CBoard pack", "footer": "&P" }
            }),
            safety: "Surgically patches only the target worksheet headerFooter/oddHeader/oddFooter. Pass null header and footer to clear. Excel codes (&C, &P) are stored verbatim (XML-escaped). Inspect surfaces header_footer when present. Other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_sheet_zoom".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Set worksheet view zoom via sheetView zoomScale (percent 10–400)."
                .into(),
            example: json!({
                "kind": "set_sheet_zoom",
                "payload": { "sheet": "Revenue", "zoom": 75 }
            }),
            safety: "Surgically patches only the target worksheet sheetView zoomScale attribute. Existing freeze-pane children and other sheetView attributes are preserved. Inspect surfaces zoom. Other sheets stay byte-identical."
                .into(),
        },
        EditCapability {
            operation: "set_show_gridlines".into(),
            schema_version: crate::edits::SCHEMA_VERSION,
            description: "Show or hide worksheet view gridlines via sheetView showGridLines."
                .into(),
            example: json!({
                "kind": "set_show_gridlines",
                "payload": { "sheet": "Revenue", "show": false }
            }),
            safety: "Surgically patches only the target worksheet sheetView showGridLines attribute. false writes showGridLines=\"0\"; true removes the attribute (Excel default shown). Existing freeze-pane children, zoomScale, and other sheetView attributes are preserved. Inspect surfaces show_gridlines. Other sheets stay byte-identical."
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
