use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use dotall_core::{
    ArtifactEnvelope, DependencyImpact, DotallError, Result, SemanticChange, SemanticOperation,
    ValidatedEdit,
};
use serde_json::Value;

use crate::dependencies::{DependencyGraph, build};
use crate::edits::impact::{ImpactOperation, validate_impact};
use crate::edits::ops::{
    DeleteSheetPolicy, EditableCell, EditableValue, SCHEMA_ID, SCHEMA_VERSION, XlsxEditOp,
    format_cell_value, format_editable_value,
};
use crate::edits::transform::{RangeRef, column_number, parse_range as parse_a1_range};
use crate::ids;
use crate::model::{CellModel, CellValue, SCHEMA_ID as MODEL_SCHEMA_ID, WorkbookModel};
use crate::selector::{self, CellAddress};
use crate::{FORMAT_ID, SCHEMA_VERSION as MODEL_SCHEMA_VERSION};

#[derive(Debug, Clone, PartialEq)]
struct ResolvedCell {
    sheet: String,
    address: String,
    element_id: String,
    row: u32,
    col: u32,
    value: CellValue,
    formula: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct ParsedOperation {
    op: XlsxEditOp,
    resolved: ResolvedCell,
}

pub fn validate(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    reject_unsupported_comment_or_chart_ops(operations)?;
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "merge_cells" | "unmerge_cells"))
    {
        return validate_merge_operations(model, operations);
    }
    if operations.iter().any(|operation| {
        matches!(
            operation.kind.as_str(),
            "set_column_width" | "set_row_height"
        )
    }) {
        return validate_dimension_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "freeze_panes"))
    {
        return validate_freeze_panes_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "insert_comment"))
    {
        return validate_insert_comment_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "insert_picture"))
    {
        return validate_insert_picture_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_tab_color"))
    {
        return validate_set_tab_color_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_auto_filter"))
    {
        return validate_set_auto_filter_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_print_area"))
    {
        return validate_set_print_area_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_print_titles"))
    {
        return validate_set_print_titles_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_page_orientation"))
    {
        return validate_set_page_orientation_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_paper_size"))
    {
        return validate_set_paper_size_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_print_scale"))
    {
        return validate_set_print_scale_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_fit_to_page"))
    {
        return validate_set_fit_to_page_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_center_on_page"))
    {
        return validate_set_center_on_page_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_page_margins"))
    {
        return validate_set_page_margins_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_header_footer"))
    {
        return validate_set_header_footer_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_cell_font"))
    {
        return validate_set_cell_font_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_cell_fill"))
    {
        return validate_set_cell_fill_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_sheet_zoom"))
    {
        return validate_set_sheet_zoom_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_show_gridlines"))
    {
        return validate_set_show_gridlines_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_right_to_left"))
    {
        return validate_set_right_to_left_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "define_name"))
    {
        return validate_define_name_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "delete_name"))
    {
        return validate_delete_name_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "hide_sheet"))
    {
        return Err(format_error(
            "hide_sheet requires source-aware validation (validate_edit_with_source)",
        ));
    }

    let workbook = decode(model)?;
    let graph = build(&workbook);
    let parsed = parse_operations(&workbook, operations)?;
    let semantic_diff = build_semantic_diff(&parsed);
    let dependency_impact = build_dependency_impact(&parsed, &graph, &workbook);

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: parsed
            .iter()
            .map(|operation| operation_to_semantic(&operation.op))
            .collect(),
        semantic_diff,
        dependency_impact,
    })
}

/// Validates operations that require package-level impact analysis before staging.
///
/// Cell edits remain model-only. Structural operations additionally inspect the
/// source OOXML package so they can reject unsupported impacted parts before the
/// transaction is journaled.
pub fn validate_with_source(
    source: &Path,
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    reject_unsupported_comment_or_chart_ops(operations)?;
    if operations.iter().any(|operation| {
        matches!(
            operation.kind.as_str(),
            "add_sheet" | "rename_sheet" | "delete_sheet"
        )
    }) {
        return validate_sheet_operation(source, model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "merge_cells" | "unmerge_cells"))
    {
        return validate_merge_operations(model, operations);
    }
    if operations.iter().any(|operation| {
        matches!(
            operation.kind.as_str(),
            "set_column_width" | "set_row_height"
        )
    }) {
        return validate_dimension_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "freeze_panes"))
    {
        return validate_freeze_panes_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "insert_comment"))
    {
        return validate_insert_comment_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "insert_picture"))
    {
        return validate_insert_picture_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_tab_color"))
    {
        return validate_set_tab_color_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_auto_filter"))
    {
        return validate_set_auto_filter_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_print_area"))
    {
        return validate_set_print_area_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_print_titles"))
    {
        return validate_set_print_titles_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_page_orientation"))
    {
        return validate_set_page_orientation_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_paper_size"))
    {
        return validate_set_paper_size_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_print_scale"))
    {
        return validate_set_print_scale_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_fit_to_page"))
    {
        return validate_set_fit_to_page_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_center_on_page"))
    {
        return validate_set_center_on_page_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_page_margins"))
    {
        return validate_set_page_margins_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_header_footer"))
    {
        return validate_set_header_footer_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_cell_font"))
    {
        return validate_set_cell_font_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_cell_fill"))
    {
        return validate_set_cell_fill_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_sheet_zoom"))
    {
        return validate_set_sheet_zoom_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_show_gridlines"))
    {
        return validate_set_show_gridlines_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "set_right_to_left"))
    {
        return validate_set_right_to_left_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "define_name"))
    {
        return validate_define_name_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "delete_name"))
    {
        return validate_delete_name_operations(model, operations);
    }
    if operations
        .iter()
        .any(|operation| matches!(operation.kind.as_str(), "hide_sheet"))
    {
        return validate_hide_sheet_operations(source, model, operations);
    }
    if operations.iter().all(|operation| {
        !matches!(
            operation.kind.as_str(),
            "insert_row" | "delete_row" | "insert_column" | "delete_column"
        )
    }) {
        return validate(model, operations);
    }
    if operations.len() != 1
        || !matches!(
            operations[0].kind.as_str(),
            "insert_row" | "delete_row" | "insert_column" | "delete_column"
        )
    {
        return Err(format_error(
            "structural edits cannot be combined with other operations",
        ));
    }

    let workbook = decode(model)?;
    let operation = &operations[0];
    let sheet = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| {
            format_error(format!(
                "{} requires a non-empty `sheet` field",
                operation.kind
            ))
        })?;
    let sheet_model = find_sheet(&workbook, sheet)?;
    let canonical_sheet = sheet_model.name.clone();
    let at = required_positive_u32(&operation.payload, "at")?;
    let count = required_positive_u32(&operation.payload, "count")?;
    let (axis, limit) = match operation.kind.as_str() {
        "insert_row" | "delete_row" => ("row", 1_048_576),
        "insert_column" | "delete_column" => ("column", 16_384),
        _ => unreachable!(),
    };
    if at > limit {
        return Err(format_error(format!(
            "structural {axis} `at` must not exceed {limit}"
        )));
    }
    if at
        .checked_add(count - 1)
        .is_none_or(|last_coordinate| last_coordinate > limit)
    {
        return Err(format_error(format!(
            "structural {axis} interval must not exceed {limit}"
        )));
    }
    let package = fs::read(source).map_err(|error| DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: source.into(),
        message: error.to_string(),
    })?;
    validate_impact(
        &package,
        &match operation.kind.as_str() {
            "insert_row" => ImpactOperation::InsertRow {
                sheet: canonical_sheet.clone(),
                at,
                count,
            },
            "delete_row" => ImpactOperation::DeleteRow {
                sheet: canonical_sheet.clone(),
                at,
                count,
            },
            "insert_column" => ImpactOperation::InsertColumn {
                sheet: canonical_sheet.clone(),
                at,
                count,
            },
            "delete_column" => ImpactOperation::DeleteColumn {
                sheet: canonical_sheet.clone(),
                at,
                count,
            },
            _ => unreachable!(),
        },
    )?;

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: operation.kind.clone(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "at": at,
                "count": count,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!{axis}:{at}"),
            element_id: format!("{axis}:{canonical_sheet}:{at}"),
            change: operation.kind.clone(),
            before: None,
            after: Some(count.to_string()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: vec!["refs parsed; values not evaluated".into()],
        },
    })
}

fn validate_sheet_operation(
    source: &Path,
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "sheet structural edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    let package = fs::read(source).map_err(|error| DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: source.into(),
        message: error.to_string(),
    })?;

    match operation.kind.as_str() {
        "add_sheet" => {
            let name = required_sheet_name(&operation.payload, "name")?;
            validate_new_sheet_name(&workbook, &name, None)?;
            let after = optional_sheet_name(&operation.payload, "after")?
                .map(|after| find_sheet(&workbook, &after).map(|sheet| sheet.name.clone()))
                .transpose()?;
            Ok(ValidatedEdit {
                format_id: FORMAT_ID.into(),
                schema_id: SCHEMA_ID.into(),
                schema_version: SCHEMA_VERSION,
                operations: vec![SemanticOperation {
                    kind: "add_sheet".into(),
                    payload: serde_json::json!({ "name": name, "after": after }),
                }],
                semantic_diff: vec![SemanticChange {
                    target: name.clone(),
                    element_id: format!("sheet:{name}"),
                    change: "add_sheet".into(),
                    before: None,
                    after: Some(after.unwrap_or_else(|| "end".into())),
                }],
                dependency_impact: DependencyImpact {
                    forward: Vec::new(),
                    notes: vec!["refs parsed; values not evaluated".into()],
                },
            })
        }
        "rename_sheet" => {
            let from = required_sheet_name(&operation.payload, "from")?;
            let to = required_sheet_name(&operation.payload, "to")?;
            let canonical_from = find_sheet(&workbook, &from)?.name.clone();
            validate_new_sheet_name(&workbook, &to, Some(&canonical_from))?;
            crate::edits::writer::validate_rename_safety(&package, &canonical_from)?;
            Ok(ValidatedEdit {
                format_id: FORMAT_ID.into(),
                schema_id: SCHEMA_ID.into(),
                schema_version: SCHEMA_VERSION,
                operations: vec![SemanticOperation {
                    kind: "rename_sheet".into(),
                    payload: serde_json::json!({ "from": canonical_from, "to": to }),
                }],
                semantic_diff: vec![SemanticChange {
                    target: canonical_from.clone(),
                    element_id: format!("sheet:{canonical_from}"),
                    change: "rename_sheet".into(),
                    before: Some(canonical_from),
                    after: Some(to),
                }],
                dependency_impact: DependencyImpact {
                    forward: Vec::new(),
                    notes: vec!["refs parsed; values not evaluated".into()],
                },
            })
        }
        "delete_sheet" => {
            let name = required_sheet_name(&operation.payload, "name")?;
            let canonical_name = find_sheet(&workbook, &name)?.name.clone();
            let dependency_policy = parse_delete_sheet_policy(&operation.payload)?;
            let references =
                crate::edits::writer::delete_sheet_references(&package, &canonical_name)?;
            if dependency_policy == DeleteSheetPolicy::RejectIfReferenced
                && (!references.formula_cells.is_empty() || !references.defined_names.is_empty())
            {
                let mut inbound = references.formula_cells;
                inbound.extend(references.defined_names);
                return Err(format_error(format!(
                    "delete_sheet `{canonical_name}` is referenced by: {}",
                    inbound.join(", ")
                )));
            }
            Ok(ValidatedEdit {
                format_id: FORMAT_ID.into(),
                schema_id: SCHEMA_ID.into(),
                schema_version: SCHEMA_VERSION,
                operations: vec![SemanticOperation {
                    kind: "delete_sheet".into(),
                    payload: serde_json::json!({
                        "name": canonical_name,
                        "dependency_policy": match dependency_policy {
                            DeleteSheetPolicy::RejectIfReferenced => "reject_if_referenced",
                            DeleteSheetPolicy::ReplaceReferencesWithRefError =>
                                "replace_references_with_ref_error",
                        },
                    }),
                }],
                semantic_diff: vec![SemanticChange {
                    target: canonical_name.clone(),
                    element_id: format!("sheet:{canonical_name}"),
                    change: "delete_sheet".into(),
                    before: Some(canonical_name),
                    after: None,
                }],
                dependency_impact: DependencyImpact {
                    forward: Vec::new(),
                    notes: vec!["references are rewritten to #REF! on apply".into()],
                },
            })
        }
        _ => Err(format_error("unsupported sheet structural edit")),
    }
}

fn parse_delete_sheet_policy(payload: &Value) -> Result<DeleteSheetPolicy> {
    match payload
        .get("dependency_policy")
        .and_then(Value::as_str)
        .unwrap_or("reject_if_referenced")
    {
        "reject_if_referenced" => Ok(DeleteSheetPolicy::RejectIfReferenced),
        "replace_references_with_ref_error" => Ok(DeleteSheetPolicy::ReplaceReferencesWithRefError),
        value => Err(format_error(format!(
            "delete_sheet has unsupported dependency_policy `{value}`"
        ))),
    }
}

fn validate_merge_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "merge edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| {
            format_error(format!(
                "{} requires a non-empty `sheet` field",
                operation.kind
            ))
        })?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let range_text = operation
        .payload
        .get("range")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|range| !range.is_empty())
        .ok_or_else(|| {
            format_error(format!(
                "{} requires a non-empty `range` field",
                operation.kind
            ))
        })?;
    let range = parse_a1_range(range_text).map_err(format_error)?;
    let canonical_range = canonicalize_merge_ref(&range);
    if range.start.row == range.end.row
        && column_number(&range.start.column) == column_number(&range.end.column)
    {
        return Err(format_error(format!(
            "{} rejects single-cell ranges (`{canonical_range}`)",
            operation.kind
        )));
    }

    match operation.kind.as_str() {
        "merge_cells" => {
            for existing in &sheet.merges {
                let existing_range = parse_a1_range(existing).map_err(|message| {
                    format_error(format!("invalid existing merge `{existing}`: {message}"))
                })?;
                if ranges_overlap(&range, &existing_range) {
                    return Err(format_error(format!(
                        "merge_cells `{canonical_range}` overlaps existing merge `{existing}` on sheet `{canonical_sheet}`"
                    )));
                }
            }
            Ok(ValidatedEdit {
                format_id: FORMAT_ID.into(),
                schema_id: SCHEMA_ID.into(),
                schema_version: SCHEMA_VERSION,
                operations: vec![SemanticOperation {
                    kind: "merge_cells".into(),
                    payload: serde_json::json!({
                        "sheet": canonical_sheet,
                        "range": canonical_range,
                    }),
                }],
                semantic_diff: vec![SemanticChange {
                    target: format!("{canonical_sheet}!{canonical_range}"),
                    element_id: format!("merge:{canonical_sheet}:{canonical_range}"),
                    change: "merge_cells".into(),
                    before: None,
                    after: Some(canonical_range),
                }],
                dependency_impact: DependencyImpact {
                    forward: Vec::new(),
                    notes: vec!["refs parsed; values not evaluated".into()],
                },
            })
        }
        "unmerge_cells" => {
            let existing = sheet
                .merges
                .iter()
                .find(|merge| {
                    parse_a1_range(merge)
                        .ok()
                        .map(|parsed| canonicalize_merge_ref(&parsed) == canonical_range)
                        .unwrap_or(false)
                })
                .cloned()
                .ok_or_else(|| {
                    format_error(format!(
                        "unmerge_cells `{canonical_range}` was not found on sheet `{canonical_sheet}`"
                    ))
                })?;
            Ok(ValidatedEdit {
                format_id: FORMAT_ID.into(),
                schema_id: SCHEMA_ID.into(),
                schema_version: SCHEMA_VERSION,
                operations: vec![SemanticOperation {
                    kind: "unmerge_cells".into(),
                    payload: serde_json::json!({
                        "sheet": canonical_sheet,
                        "range": canonicalize_merge_ref(
                            &parse_a1_range(&existing).map_err(format_error)?,
                        ),
                    }),
                }],
                semantic_diff: vec![SemanticChange {
                    target: format!("{canonical_sheet}!{canonical_range}"),
                    element_id: format!("merge:{canonical_sheet}:{canonical_range}"),
                    change: "unmerge_cells".into(),
                    before: Some(canonical_range),
                    after: None,
                }],
                dependency_impact: DependencyImpact {
                    forward: Vec::new(),
                    notes: vec!["refs parsed; values not evaluated".into()],
                },
            })
        }
        _ => Err(format_error("unsupported merge edit")),
    }
}

fn canonicalize_merge_ref(range: &RangeRef) -> String {
    format!(
        "{}{}:{}{}",
        range.start.column, range.start.row, range.end.column, range.end.row
    )
}

fn ranges_overlap(left: &RangeRef, right: &RangeRef) -> bool {
    let left_start_col = column_number(&left.start.column);
    let left_end_col = column_number(&left.end.column);
    let right_start_col = column_number(&right.start.column);
    let right_end_col = column_number(&right.end.column);
    !(left.end.row < right.start.row
        || right.end.row < left.start.row
        || left_end_col < right_start_col
        || right_end_col < left_start_col)
}

fn validate_dimension_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "dimension edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| {
            format_error(format!(
                "{} requires a non-empty `sheet` field",
                operation.kind
            ))
        })?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();

    match operation.kind.as_str() {
        "set_column_width" => {
            let column_text = operation
                .payload
                .get("column")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|column| !column.is_empty())
                .ok_or_else(|| {
                    format_error("set_column_width requires a non-empty `column` field")
                })?;
            let column = canonicalize_column(column_text)?;
            let width = required_positive_dimension(operation, "width")?;
            if width > 255.0 {
                return Err(format_error("set_column_width `width` must be at most 255"));
            }
            Ok(ValidatedEdit {
                format_id: FORMAT_ID.into(),
                schema_id: SCHEMA_ID.into(),
                schema_version: SCHEMA_VERSION,
                operations: vec![SemanticOperation {
                    kind: "set_column_width".into(),
                    payload: serde_json::json!({
                        "sheet": canonical_sheet,
                        "column": column,
                        "width": width,
                    }),
                }],
                semantic_diff: vec![SemanticChange {
                    target: format!("{canonical_sheet}!{column}"),
                    element_id: format!("col:{canonical_sheet}:{column}"),
                    change: "set_column_width".into(),
                    before: None,
                    after: Some(format_dimension(width)),
                }],
                dependency_impact: DependencyImpact {
                    forward: Vec::new(),
                    notes: vec!["refs parsed; values not evaluated".into()],
                },
            })
        }
        "set_row_height" => {
            let row = operation
                .payload
                .get("row")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| format_error("set_row_height requires a positive `row` field"))?;
            if row > crate::edits::transform::MAX_ROWS {
                return Err(format_error(format!(
                    "set_row_height `row` must be between 1 and {}",
                    crate::edits::transform::MAX_ROWS
                )));
            }
            let height = required_positive_dimension(operation, "height")?;
            if height > 409.0 {
                return Err(format_error("set_row_height `height` must be at most 409"));
            }
            Ok(ValidatedEdit {
                format_id: FORMAT_ID.into(),
                schema_id: SCHEMA_ID.into(),
                schema_version: SCHEMA_VERSION,
                operations: vec![SemanticOperation {
                    kind: "set_row_height".into(),
                    payload: serde_json::json!({
                        "sheet": canonical_sheet,
                        "row": row,
                        "height": height,
                    }),
                }],
                semantic_diff: vec![SemanticChange {
                    target: format!("{canonical_sheet}!R{row}"),
                    element_id: format!("row:{canonical_sheet}:{row}"),
                    change: "set_row_height".into(),
                    before: None,
                    after: Some(format_dimension(height)),
                }],
                dependency_impact: DependencyImpact {
                    forward: Vec::new(),
                    notes: vec!["refs parsed; values not evaluated".into()],
                },
            })
        }
        _ => Err(format_error("unsupported dimension edit")),
    }
}

fn canonicalize_column(column: &str) -> Result<String> {
    let upper = column.to_ascii_uppercase();
    if upper.is_empty()
        || !upper.bytes().all(|byte| byte.is_ascii_uppercase())
        || column_number(&upper) == 0
        || column_number(&upper) > crate::edits::transform::MAX_COLUMNS
    {
        return Err(format_error(format!(
            "set_column_width `column` must be a valid Excel column letter (got `{column}`)"
        )));
    }
    Ok(upper)
}

fn required_positive_dimension(operation: &SemanticOperation, field: &str) -> Result<f64> {
    operation
        .payload
        .get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| format_error(format!("{} requires a positive `{field}`", operation.kind)))
}

fn format_dimension(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() <= i64::MAX as f64 {
        format!("{}", value as i64)
    } else {
        let formatted = format!("{value}");
        formatted
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn validate_freeze_panes_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "freeze_panes edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "freeze_panes" {
        return Err(format_error("unsupported freeze panes edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("freeze_panes requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.freeze_panes.clone();

    let cell = match operation.payload.get("cell") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Err(format_error(
                    "freeze_panes `cell` must be a non-empty cell address when present",
                ));
            }
            let address = parse_address(trimmed)?;
            let canonical = format_address(address);
            if address.row == 1 && address.col == 1 {
                None
            } else {
                Some(canonical)
            }
        }
        _ => {
            return Err(format_error(
                "freeze_panes `cell` must be a cell address string or null",
            ));
        }
    };

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "freeze_panes".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "cell": cell,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!freeze_panes"),
            element_id: format!("freeze:{canonical_sheet}"),
            change: "freeze_panes".into(),
            before,
            after: cell.clone(),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: vec!["refs parsed; values not evaluated".into()],
        },
    })
}

fn reject_unsupported_comment_or_chart_ops(operations: &[SemanticOperation]) -> Result<()> {
    for operation in operations {
        let kind = operation.kind.as_str();
        if matches!(
            kind,
            "set_comment"
                | "delete_comment"
                | "replace_comment"
                | "update_comment"
                | "set_picture"
                | "delete_picture"
                | "replace_picture"
        ) {
            return Err(unsupported_edit_capability(kind));
        }
        if kind.contains("chart") && kind != "insert_comment" {
            return Err(unsupported_edit_capability(kind));
        }
    }
    Ok(())
}

fn unsupported_edit_capability(kind: &str) -> DotallError {
    DotallError::UnsupportedCapability {
        format_id: FORMAT_ID.into(),
        capability: kind.into(),
        available: vec![
            "insert_comment".into(),
            "insert_picture".into(),
            "set_cell_value".into(),
            "set_cell_formula".into(),
            "set_range".into(),
            "freeze_panes".into(),
        ],
    }
}

fn validate_insert_comment_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "insert_comment edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "insert_comment" {
        return Err(format_error("unsupported insert_comment edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("insert_comment requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let address_raw = operation
        .payload
        .get("address")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .ok_or_else(|| format_error("insert_comment requires a non-empty `address` field"))?;
    let address = format_address(parse_address(address_raw)?);
    let text = operation
        .payload
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format_error("insert_comment requires a non-empty `text` field"))?;
    let author = match operation.payload.get("author") {
        None | Some(Value::Null) => "Dotall".to_owned(),
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Err(format_error(
                    "insert_comment `author` must be non-empty when present",
                ));
            }
            trimmed.to_owned()
        }
        _ => {
            return Err(format_error(
                "insert_comment `author` must be a string when present",
            ));
        }
    };

    let cell_exists = sheet
        .cells
        .iter()
        .any(|cell| cell.address.eq_ignore_ascii_case(&address));
    if !cell_exists {
        return Err(format_error(format!(
            "insert_comment requires an existing cell `{canonical_sheet}!{address}`"
        )));
    }
    if workbook.comments.iter().any(|comment| {
        comment.sheet.eq_ignore_ascii_case(&canonical_sheet)
            && comment.cell.eq_ignore_ascii_case(&address)
    }) {
        return Err(format_error(format!(
            "insert_comment rejected: cell `{canonical_sheet}!{address}` already has a comment"
        )));
    }

    let element_id = ids::comment_id(&canonical_sheet, &address, MODEL_SCHEMA_VERSION);
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "insert_comment".into(),
            payload: serde_json::json!({
                "sheet": &canonical_sheet,
                "address": &address,
                "element_id": &element_id,
                "text": &text,
                "author": &author,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!{address}"),
            element_id,
            change: "insert_comment".into(),
            before: None,
            after: Some(text),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_insert_picture_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    use base64::Engine;

    if operations.len() != 1 {
        return Err(format_error(
            "insert_picture edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "insert_picture" {
        return Err(format_error("unsupported insert_picture edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("insert_picture requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let from_cell_raw = operation
        .payload
        .get("from_cell")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .ok_or_else(|| format_error("insert_picture requires a non-empty `from_cell` field"))?;
    let from_cell = format_address(parse_address(from_cell_raw)?);
    let content_type = match operation.payload.get("content_type") {
        None | Some(Value::Null) => "image/png".to_owned(),
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed != "image/png" && trimmed != "image/jpeg" {
                return Err(format_error(
                    "insert_picture `content_type` must be image/png or image/jpeg",
                ));
            }
            trimmed.to_owned()
        }
        _ => {
            return Err(format_error(
                "insert_picture `content_type` must be a string when present",
            ));
        }
    };
    let encoded = operation
        .payload
        .get("bytes_base64")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format_error("insert_picture requires a non-empty `bytes_base64` field"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format_error(format!("insert_picture invalid base64: {error}")))?;
    if bytes.is_empty() {
        return Err(format_error("insert_picture rejects empty image bytes"));
    }

    let element_id = ids::picture_id(
        &canonical_sheet,
        &format!("pending:{from_cell}"),
        MODEL_SCHEMA_VERSION,
    );
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "insert_picture".into(),
            payload: serde_json::json!({
                "sheet": &canonical_sheet,
                "from_cell": &from_cell,
                "bytes_base64": encoded,
                "content_type": &content_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!{from_cell}"),
            element_id,
            change: "insert_picture".into(),
            before: None,
            after: Some(content_type),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn normalize_tab_color(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_start_matches('#').to_ascii_uppercase();
    let hex = if trimmed.len() == 6 {
        format!("FF{trimmed}")
    } else {
        trimmed
    };
    if hex.len() != 8 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format_error(
            "set_tab_color `color` must be a 6- or 8-digit RGB/AARRGGBB hex string",
        ));
    }
    Ok(hex)
}

fn validate_set_tab_color_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_tab_color edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_tab_color" {
        return Err(format_error("unsupported set_tab_color edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_tab_color requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.tab_color.clone();
    let color = match operation.payload.get("color") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(normalize_tab_color(value)?),
        _ => {
            return Err(format_error(
                "set_tab_color `color` must be a hex string or null",
            ));
        }
    };

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_tab_color".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "color": color,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!tab_color"),
            element_id: format!("tab_color:{canonical_sheet}"),
            change: "set_tab_color".into(),
            before,
            after: color.clone(),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn normalize_auto_filter_range(raw: &str) -> Result<String> {
    let range = crate::edits::transform::parse_range(raw.trim()).map_err(|message| {
        format_error(format!("set_auto_filter `range` is invalid: {message}"))
    })?;
    Ok(format!(
        "{}{}:{}{}",
        range.start.column, range.start.row, range.end.column, range.end.row
    ))
}

fn normalize_print_area_range(raw: &str) -> Result<String> {
    let range = crate::edits::transform::parse_range(raw.trim())
        .map_err(|message| format_error(format!("set_print_area `range` is invalid: {message}")))?;
    Ok(format!(
        "{}{}:{}{}",
        range.start.column, range.start.row, range.end.column, range.end.row
    ))
}

fn validate_set_auto_filter_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_auto_filter edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_auto_filter" {
        return Err(format_error("unsupported set_auto_filter edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_auto_filter requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.auto_filter.clone();
    let range = match operation.payload.get("range") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(normalize_auto_filter_range(value)?),
        _ => {
            return Err(format_error(
                "set_auto_filter `range` must be an A1 range string or null",
            ));
        }
    };

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_auto_filter".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "range": range,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!auto_filter"),
            element_id: format!("auto_filter:{canonical_sheet}"),
            change: "set_auto_filter".into(),
            before,
            after: range.clone(),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_print_area_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_print_area edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_print_area" {
        return Err(format_error("unsupported set_print_area edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_print_area requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.print_area.clone();
    let range = match operation.payload.get("range") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(normalize_print_area_range(value)?),
        _ => {
            return Err(format_error(
                "set_print_area `range` must be an A1 range string or null",
            ));
        }
    };

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_print_area".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "range": range,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!print_area"),
            element_id: format!("print_area:{canonical_sheet}"),
            change: "set_print_area".into(),
            before,
            after: range.clone(),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn normalize_print_title_rows(raw: &str) -> Result<String> {
    let trimmed = raw.trim().to_ascii_uppercase();
    let (start, end) = trimmed
        .split_once(':')
        .ok_or_else(|| format_error("set_print_titles `rows` must look like `1:1`"))?;
    let start_row: u32 = start.parse().map_err(|_| {
        format_error("set_print_titles `rows` is invalid: expected numeric row span")
    })?;
    let end_row: u32 = end.parse().map_err(|_| {
        format_error("set_print_titles `rows` is invalid: expected numeric row span")
    })?;
    if start_row == 0 || end_row == 0 || start_row > end_row {
        return Err(format_error(
            "set_print_titles `rows` is invalid: row span must be 1-based and ascending",
        ));
    }
    Ok(format!("{start_row}:{end_row}"))
}

fn normalize_print_title_cols(raw: &str) -> Result<String> {
    let trimmed = raw.trim().to_ascii_uppercase();
    let (start, end) = trimmed
        .split_once(':')
        .ok_or_else(|| format_error("set_print_titles `cols` must look like `A:A`"))?;
    if start.is_empty()
        || end.is_empty()
        || !start.chars().all(|c| c.is_ascii_uppercase())
        || !end.chars().all(|c| c.is_ascii_uppercase())
    {
        return Err(format_error(
            "set_print_titles `cols` is invalid: expected column letters like `A:B`",
        ));
    }
    // Lexicographic compare is wrong for AA vs B; compare via column indices.
    let start_idx = column_letters_to_index(start)?;
    let end_idx = column_letters_to_index(end)?;
    if start_idx > end_idx {
        return Err(format_error(
            "set_print_titles `cols` is invalid: column span must be ascending",
        ));
    }
    Ok(format!("{start}:{end}"))
}

fn column_letters_to_index(letters: &str) -> Result<u32> {
    let mut value = 0_u32;
    for ch in letters.chars() {
        if !ch.is_ascii_uppercase() {
            return Err(format_error(
                "set_print_titles `cols` is invalid: expected column letters",
            ));
        }
        value = value
            .checked_mul(26)
            .and_then(|v| v.checked_add((ch as u32) - ('A' as u32) + 1))
            .ok_or_else(|| format_error("set_print_titles `cols` is invalid"))?;
    }
    if value == 0 {
        return Err(format_error("set_print_titles `cols` is invalid"));
    }
    Ok(value)
}

fn optional_print_title_field(
    payload: &Value,
    key: &str,
    normalize: fn(&str) -> Result<String>,
) -> Result<Option<String>> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(normalize(value)?)),
        _ => Err(format_error(format!(
            "set_print_titles `{key}` must be a string or null"
        ))),
    }
}

fn validate_set_print_titles_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_print_titles edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_print_titles" {
        return Err(format_error("unsupported set_print_titles edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_print_titles requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.print_titles.clone().map(|titles| {
        format!(
            "rows={},cols={}",
            titles.rows.as_deref().unwrap_or(""),
            titles.cols.as_deref().unwrap_or("")
        )
    });
    let rows = optional_print_title_field(&operation.payload, "rows", normalize_print_title_rows)?;
    let cols = optional_print_title_field(&operation.payload, "cols", normalize_print_title_cols)?;
    let after = if rows.is_none() && cols.is_none() {
        None
    } else {
        Some(format!(
            "rows={},cols={}",
            rows.as_deref().unwrap_or(""),
            cols.as_deref().unwrap_or("")
        ))
    };

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_print_titles".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "rows": rows,
                "cols": cols,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!print_titles"),
            element_id: format!("print_titles:{canonical_sheet}"),
            change: "set_print_titles".into(),
            before,
            after,
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_page_orientation_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_page_orientation edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_page_orientation" {
        return Err(format_error("unsupported set_page_orientation edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_page_orientation requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.page_orientation.clone();
    let orientation = operation
        .payload
        .get("orientation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format_error("set_page_orientation requires `orientation` of portrait or landscape")
        })?;
    let orientation = match orientation.to_ascii_lowercase().as_str() {
        "portrait" => "portrait".to_owned(),
        "landscape" => "landscape".to_owned(),
        _ => {
            return Err(format_error(
                "set_page_orientation `orientation` must be portrait or landscape",
            ));
        }
    };

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_page_orientation".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "orientation": orientation,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!page_orientation"),
            element_id: format!("page_orientation:{canonical_sheet}"),
            change: "set_page_orientation".into(),
            before,
            after: Some(orientation),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_paper_size_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_paper_size edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_paper_size" {
        return Err(format_error("unsupported set_paper_size edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_paper_size requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.paper_size;
    let paper_size = operation
        .payload
        .get("paper_size")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_f64().map(|f| f as u64))
                .and_then(|v| u32::try_from(v).ok())
        })
        .filter(|v| *v > 0)
        .ok_or_else(|| format_error("set_paper_size requires a positive integer `paper_size`"))?;

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_paper_size".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "paper_size": paper_size,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!paper_size"),
            element_id: format!("paper_size:{canonical_sheet}"),
            change: "set_paper_size".into(),
            before: before.map(|v| v.to_string()),
            after: Some(paper_size.to_string()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_print_scale_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_print_scale edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_print_scale" {
        return Err(format_error("unsupported set_print_scale edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_print_scale requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.print_scale;
    let scale = operation
        .payload
        .get("scale")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_f64().map(|f| f as u64))
                .and_then(|v| u32::try_from(v).ok())
        })
        .filter(|scale| (10..=400).contains(scale))
        .ok_or_else(|| {
            format_error("set_print_scale requires `scale` integer between 10 and 400")
        })?;

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_print_scale".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "scale": scale,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!print_scale"),
            element_id: format!("print_scale:{canonical_sheet}"),
            change: "set_print_scale".into(),
            before: Some(match before {
                Some(value) => value.to_string(),
                None => "default".into(),
            }),
            after: Some(scale.to_string()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_fit_to_page_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_fit_to_page edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_fit_to_page" {
        return Err(format_error("unsupported set_fit_to_page edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_fit_to_page requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.fit_to_page.clone();
    let width = optional_fit_dim(&operation.payload, "width")?;
    let height = optional_fit_dim(&operation.payload, "height")?;
    match (width, height) {
        (Some(_), Some(_)) | (None, None) => {}
        _ => {
            return Err(format_error(
                "set_fit_to_page requires both `width` and `height`, or both null to clear",
            ));
        }
    }

    let after = match (width, height) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        (None, None) => "cleared".into(),
        _ => unreachable!(),
    };
    let before_text = match before {
        Some(fit) => format!(
            "{}x{}",
            fit.width
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            fit.height
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into())
        ),
        None => "unset".into(),
    };

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_fit_to_page".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "width": width,
                "height": height,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!fit_to_page"),
            element_id: format!("fit_to_page:{canonical_sheet}"),
            change: "set_fit_to_page".into(),
            before: Some(before_text),
            after: Some(after),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn optional_fit_dim(payload: &Value, field: &str) -> Result<Option<u32>> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .or_else(|| value.as_f64().map(|f| f as u64))
            .and_then(|v| u32::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| {
                format_error(format!(
                    "set_fit_to_page `{field}` must be a non-negative integer when present"
                ))
            }),
    }
}

fn validate_set_center_on_page_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_center_on_page edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_center_on_page" {
        return Err(format_error("unsupported set_center_on_page edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_center_on_page requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.center_on_page.clone();
    let horizontal = operation
        .payload
        .get("horizontal")
        .and_then(Value::as_bool)
        .ok_or_else(|| format_error("set_center_on_page requires `horizontal` boolean"))?;
    let vertical = operation
        .payload
        .get("vertical")
        .and_then(Value::as_bool)
        .ok_or_else(|| format_error("set_center_on_page requires `vertical` boolean"))?;
    let after = if horizontal || vertical {
        format!(
            "h={} v={}",
            if horizontal { "1" } else { "0" },
            if vertical { "1" } else { "0" }
        )
    } else {
        "cleared".into()
    };
    let before_text = match before {
        Some(center) => format!(
            "h={} v={}",
            if center.horizontal { "1" } else { "0" },
            if center.vertical { "1" } else { "0" }
        ),
        None => "unset".into(),
    };

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_center_on_page".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "horizontal": horizontal,
                "vertical": vertical,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!center_on_page"),
            element_id: format!("center_on_page:{canonical_sheet}"),
            change: "set_center_on_page".into(),
            before: Some(before_text),
            after: Some(after),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_page_margins_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_page_margins edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_page_margins" {
        return Err(format_error("unsupported set_page_margins edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_page_margins requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet
        .page_margins
        .as_ref()
        .map(|margins| {
            format!(
                "L{} R{} T{} B{}",
                margins.left, margins.right, margins.top, margins.bottom
            )
        })
        .unwrap_or_else(|| "default".into());
    let left = required_non_negative_margin(&operation.payload, "left")?;
    let right = required_non_negative_margin(&operation.payload, "right")?;
    let top = required_non_negative_margin(&operation.payload, "top")?;
    let bottom = required_non_negative_margin(&operation.payload, "bottom")?;
    let header = optional_non_negative_margin(&operation.payload, "header")?;
    let footer = optional_non_negative_margin(&operation.payload, "footer")?;
    let after = format!("L{left} R{right} T{top} B{bottom}");

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_page_margins".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "left": left,
                "right": right,
                "top": top,
                "bottom": bottom,
                "header": header,
                "footer": footer,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!page_margins"),
            element_id: format!("page_margins:{canonical_sheet}"),
            change: "set_page_margins".into(),
            before: Some(before),
            after: Some(after),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_header_footer_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_header_footer edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_header_footer" {
        return Err(format_error("unsupported set_header_footer edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_header_footer requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.header_footer.clone();
    let header = optional_header_footer_text(&operation.payload, "header")?;
    let footer = optional_header_footer_text(&operation.payload, "footer")?;
    let after = match (&header, &footer) {
        (None, None) => "cleared".into(),
        _ => format!(
            "header={} footer={}",
            header.as_deref().unwrap_or(""),
            footer.as_deref().unwrap_or("")
        ),
    };
    let before_text = match before {
        Some(value) => format!(
            "header={} footer={}",
            value.header.as_deref().unwrap_or(""),
            value.footer.as_deref().unwrap_or("")
        ),
        None => "unset".into(),
    };

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_header_footer".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "header": header,
                "footer": footer,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!header_footer"),
            element_id: format!("header_footer:{canonical_sheet}"),
            change: "set_header_footer".into(),
            before: Some(before_text),
            after: Some(after),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_cell_font_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_cell_font edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_cell_font" {
        return Err(format_error("unsupported set_cell_font edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_cell_font requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let address_raw = operation
        .payload
        .get("address")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .ok_or_else(|| format_error("set_cell_font requires a non-empty `address` field"))?;
    let address = format_address(parse_address(address_raw)?);
    let bold = optional_bool_field(&operation.payload, "bold")?;
    let italic = optional_bool_field(&operation.payload, "italic")?;
    let name = optional_non_empty_string_field(&operation.payload, "name")?;
    let size_pt = optional_positive_f64_field(&operation.payload, "size_pt")?;
    let color = match operation.payload.get("color") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(normalize_tab_color(value).map_err(|_| {
            format_error("set_cell_font `color` must be a 6- or 8-digit RGB/AARRGGBB hex string")
        })?),
        _ => {
            return Err(format_error(
                "set_cell_font `color` must be a hex string when present",
            ));
        }
    };
    if bold.is_none() && italic.is_none() && name.is_none() && size_pt.is_none() && color.is_none()
    {
        return Err(format_error(
            "set_cell_font requires at least one font field (`bold`, `italic`, `name`, `size_pt`, or `color`)",
        ));
    }

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_cell_font".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "address": address,
                "bold": bold,
                "italic": italic,
                "name": name,
                "size_pt": size_pt,
                "color": color,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!{address}"),
            element_id: ids::cell_id(&canonical_sheet, &address, MODEL_SCHEMA_VERSION),
            change: "set_cell_font".into(),
            before: None,
            after: Some(format!(
                "bold={:?} italic={:?} name={:?} size_pt={:?} color={:?}",
                bold, italic, name, size_pt, color
            )),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_cell_fill_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_cell_fill edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_cell_fill" {
        return Err(format_error("unsupported set_cell_fill edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_cell_fill requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let address_raw = operation
        .payload
        .get("address")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .ok_or_else(|| format_error("set_cell_fill requires a non-empty `address` field"))?;
    let address = format_address(parse_address(address_raw)?);
    if !operation
        .payload
        .as_object()
        .is_some_and(|obj| obj.contains_key("color"))
    {
        return Err(format_error(
            "set_cell_fill requires a `color` field (hex string or null)",
        ));
    }
    let color = match operation.payload.get("color") {
        Some(Value::Null) => None,
        Some(Value::String(value)) => Some(normalize_tab_color(value).map_err(|_| {
            format_error("set_cell_fill `color` must be a 6- or 8-digit RGB/AARRGGBB hex string")
        })?),
        _ => {
            return Err(format_error(
                "set_cell_fill `color` must be a hex string or null",
            ));
        }
    };

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_cell_fill".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "address": address,
                "color": color,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!{address}"),
            element_id: ids::cell_id(&canonical_sheet, &address, MODEL_SCHEMA_VERSION),
            change: "set_cell_fill".into(),
            before: None,
            after: color.clone().or_else(|| Some("cleared".into())),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn optional_bool_field(payload: &Value, field: &str) -> Result<Option<bool>> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(format_error(format!(
            "set_cell_font `{field}` must be a boolean when present"
        ))),
    }
}

fn optional_non_empty_string_field(payload: &Value, field: &str) -> Result<Option<String>> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.trim().to_owned())),
        Some(Value::String(_)) => Err(format_error(format!(
            "set_cell_font `{field}` must be a non-empty string when present"
        ))),
        _ => Err(format_error(format!(
            "set_cell_font `{field}` must be a string when present"
        ))),
    }
}

fn optional_positive_f64_field(payload: &Value, field: &str) -> Result<Option<f64>> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(Some)
            .ok_or_else(|| {
                format_error(format!(
                    "set_cell_font `{field}` must be a positive number when present"
                ))
            }),
    }
}

fn validate_set_sheet_zoom_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_sheet_zoom edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_sheet_zoom" {
        return Err(format_error("unsupported set_sheet_zoom edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_sheet_zoom requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.zoom;
    let zoom = operation
        .payload
        .get("zoom")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_f64().map(|f| f as u64))
                .and_then(|v| u32::try_from(v).ok())
        })
        .filter(|zoom| (10..=400).contains(zoom))
        .ok_or_else(|| format_error("set_sheet_zoom requires `zoom` integer between 10 and 400"))?;

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_sheet_zoom".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "zoom": zoom,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!zoom"),
            element_id: format!("zoom:{canonical_sheet}"),
            change: "set_sheet_zoom".into(),
            before: before.map(|v| v.to_string()),
            after: Some(zoom.to_string()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_show_gridlines_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_show_gridlines edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_show_gridlines" {
        return Err(format_error("unsupported set_show_gridlines edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_show_gridlines requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.show_gridlines;
    let show = operation
        .payload
        .get("show")
        .and_then(Value::as_bool)
        .ok_or_else(|| format_error("set_show_gridlines requires boolean `show`"))?;

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_show_gridlines".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "show": show,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!show_gridlines"),
            element_id: format!("show_gridlines:{canonical_sheet}"),
            change: "set_show_gridlines".into(),
            before: Some(before.to_string()),
            after: Some(show.to_string()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_right_to_left_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "set_right_to_left edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "set_right_to_left" {
        return Err(format_error("unsupported set_right_to_left edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_right_to_left requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let canonical_sheet = sheet.name.clone();
    let before = sheet.right_to_left;
    let rtl = operation
        .payload
        .get("rtl")
        .and_then(Value::as_bool)
        .ok_or_else(|| format_error("set_right_to_left requires boolean `rtl`"))?;

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_right_to_left".into(),
            payload: serde_json::json!({
                "sheet": canonical_sheet,
                "rtl": rtl,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{canonical_sheet}!right_to_left"),
            element_id: format!("right_to_left:{canonical_sheet}"),
            change: "set_right_to_left".into(),
            before: Some(before.to_string()),
            after: Some(rtl.to_string()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn optional_header_footer_text(payload: &Value, field: &str) -> Result<Option<String>> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(format_error(format!(
            "set_header_footer `{field}` must be a string or null when present"
        ))),
    }
}

fn required_non_negative_margin(payload: &Value, field: &str) -> Result<f64> {
    payload
        .get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| {
            format_error(format!(
                "set_page_margins requires non-negative `{field}` margin in inches"
            ))
        })
}

fn optional_non_negative_margin(payload: &Value, field: &str) -> Result<Option<f64>> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(Some)
            .ok_or_else(|| {
                format_error(format!(
                    "set_page_margins `{field}` must be a non-negative number when present"
                ))
            }),
    }
}

fn validate_define_name_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "define_name edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "define_name" {
        return Err(format_error("unsupported define_name edit"));
    }
    let name = operation
        .payload
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format_error("define_name requires a non-empty `name` field"))?
        .to_owned();
    let formula_raw = operation
        .payload
        .get("formula")
        .and_then(Value::as_str)
        .ok_or_else(|| format_error("define_name requires a `formula` field"))?;
    let formula = normalize_defined_name_formula(formula_raw)?;
    let existing = workbook
        .named_ranges
        .iter()
        .find(|range| range.name.eq_ignore_ascii_case(&name));
    let canonical_name = existing
        .map(|range| range.name.clone())
        .unwrap_or_else(|| name.clone());
    let before = existing.map(|range| range.formula.clone());
    let element_id = existing
        .map(|range| range.element_id.clone())
        .unwrap_or_else(|| ids::named_range_id(&canonical_name, &formula, MODEL_SCHEMA_VERSION));

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "define_name".into(),
            payload: serde_json::json!({
                "name": canonical_name,
                "formula": formula,
                "element_id": element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("name:{canonical_name}"),
            element_id,
            change: "define_name".into(),
            before,
            after: Some(formula),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: vec!["refs parsed; values not evaluated".into()],
        },
    })
}

fn validate_delete_name_operations(
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "delete_name edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "delete_name" {
        return Err(format_error("unsupported delete_name edit"));
    }
    let name = operation
        .payload
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format_error("delete_name requires a non-empty `name` field"))?
        .to_owned();
    let existing = workbook
        .named_ranges
        .iter()
        .find(|range| range.name.eq_ignore_ascii_case(&name))
        .ok_or_else(|| format_error(format!("named range `{name}` was not found")))?;

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "delete_name".into(),
            payload: serde_json::json!({
                "name": existing.name,
                "element_id": existing.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("name:{}", existing.name),
            element_id: existing.element_id.clone(),
            change: "delete_name".into(),
            before: Some(existing.formula.clone()),
            after: None,
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_hide_sheet_operations(
    source: &Path,
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "hide_sheet edits cannot be combined with other operations",
        ));
    }
    let workbook = decode(model)?;
    let operation = &operations[0];
    if operation.kind != "hide_sheet" {
        return Err(format_error("unsupported hide_sheet edit"));
    }
    let sheet_name = operation
        .payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format_error("hide_sheet requires a non-empty `sheet` field"))?;
    let hidden = operation
        .payload
        .get("hidden")
        .and_then(Value::as_bool)
        .ok_or_else(|| format_error("hide_sheet requires a boolean `hidden` field"))?;
    let sheet = find_sheet(&workbook, sheet_name)?;
    let package = fs::read(source).map_err(|error| DotallError::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    let visibility = crate::edits::writer::sheet_visibility(&package)?;
    let current_hidden = visibility
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&sheet.name))
        .map(|(_, is_hidden)| *is_hidden)
        .ok_or_else(|| {
            format_error(format!(
                "worksheet `{}` was not found in package",
                sheet.name
            ))
        })?;
    if hidden && !current_hidden {
        let visible_count = visibility
            .iter()
            .filter(|(_, is_hidden)| !*is_hidden)
            .count();
        if visible_count <= 1 {
            return Err(format_error("cannot hide the last visible worksheet"));
        }
    }

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "hide_sheet".into(),
            payload: serde_json::json!({
                "sheet": sheet.name,
                "hidden": hidden,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("sheet:{}", sheet.name),
            element_id: sheet.element_id.clone(),
            change: "hide_sheet".into(),
            before: Some(if current_hidden { "hidden" } else { "visible" }.into()),
            after: Some(if hidden { "hidden" } else { "visible" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn normalize_defined_name_formula(formula: &str) -> Result<String> {
    let trimmed = formula.trim();
    let body = trimmed.strip_prefix('=').unwrap_or(trimmed).trim();
    if body.is_empty() {
        return Err(format_error(
            "define_name requires a non-empty formula body",
        ));
    }
    Ok(body.to_owned())
}

fn decode(model: &ArtifactEnvelope) -> Result<WorkbookModel> {
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

fn parse_operations(
    workbook: &WorkbookModel,
    operations: &[SemanticOperation],
) -> Result<Vec<ParsedOperation>> {
    if operations.is_empty() {
        return Err(format_error("at least one edit operation is required"));
    }

    let mut parsed = operations
        .iter()
        .map(|operation| parse_operation(workbook, operation))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    parsed.sort_by(|left, right| {
        (
            left.resolved.sheet.as_str(),
            left.resolved.row,
            left.resolved.col,
        )
            .cmp(&(
                right.resolved.sheet.as_str(),
                right.resolved.row,
                right.resolved.col,
            ))
    });

    let mut seen = BTreeSet::new();
    for operation in &parsed {
        if !seen.insert(operation.resolved.element_id.clone()) {
            return Err(format_error(format!(
                "multiple operations target the same cell `{}` ({})",
                operation.resolved.element_id,
                operation.resolved.target_label()
            )));
        }
    }

    Ok(parsed)
}

fn parse_operation(
    workbook: &WorkbookModel,
    operation: &SemanticOperation,
) -> Result<Vec<ParsedOperation>> {
    match operation.kind.as_str() {
        "set_cell_value" => {
            let (resolved, value) = parse_cell_target(workbook, &operation.payload)?;
            Ok(vec![ParsedOperation {
                op: XlsxEditOp::SetCellValue {
                    sheet: resolved.sheet.clone(),
                    address: resolved.address.clone(),
                    element_id: resolved.element_id.clone(),
                    value,
                },
                resolved,
            }])
        }
        "set_cell_formula" => {
            let (resolved, formula) = parse_formula_target(workbook, &operation.payload)?;
            let formula = normalize_formula(&formula);
            Ok(vec![ParsedOperation {
                op: XlsxEditOp::SetCellFormula {
                    sheet: resolved.sheet.clone(),
                    address: resolved.address.clone(),
                    element_id: resolved.element_id.clone(),
                    formula,
                },
                resolved,
            }])
        }
        "set_range" => parse_range(workbook, &operation.payload),
        kind => Err(format_error(format!(
            "unsupported edit operation `{kind}`; supported: set_cell_value, set_cell_formula, set_range"
        ))),
    }
}

fn parse_range(workbook: &WorkbookModel, payload: &Value) -> Result<Vec<ParsedOperation>> {
    const MAX_RANGE_CELLS: usize = 10_000;
    const EXCEL_MAX_ROWS: u32 = 1_048_576;
    const EXCEL_MAX_COLS: u32 = 16_384;

    let sheet_name = payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sheet| !sheet.is_empty())
        .ok_or_else(|| format_error("set_range requires a non-empty `sheet` field"))?;
    let sheet = find_sheet(workbook, sheet_name)?;
    let start_cell = payload
        .get("start_cell")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .ok_or_else(|| format_error("set_range requires a non-empty `start_cell` field"))?;
    let start = parse_address(start_cell)?;
    let rows = payload
        .get("values")
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| format_error("set_range requires a non-empty `values` matrix"))?;
    let width = rows
        .first()
        .and_then(Value::as_array)
        .filter(|row| !row.is_empty())
        .map(Vec::len)
        .ok_or_else(|| format_error("set_range rejects empty rows"))?;
    if rows.iter().any(|row| {
        row.as_array()
            .is_none_or(|cells| cells.is_empty() || cells.len() != width)
    }) {
        return Err(format_error(
            "set_range rejects empty rows and ragged matrices",
        ));
    }
    let cell_count = rows
        .len()
        .checked_mul(width)
        .ok_or_else(|| format_error("set_range exceeds the maximum cell count"))?;
    if cell_count > MAX_RANGE_CELLS {
        return Err(format_error(format!(
            "set_range exceeds the maximum cell count of {MAX_RANGE_CELLS}"
        )));
    }
    let end_row = start
        .row
        .checked_add((rows.len() - 1) as u32)
        .ok_or_else(|| format_error("set_range exceeds Excel limits"))?;
    let end_col = start
        .col
        .checked_add((width - 1) as u32)
        .ok_or_else(|| format_error("set_range exceeds Excel limits"))?;
    if end_row > EXCEL_MAX_ROWS || end_col > EXCEL_MAX_COLS {
        return Err(format_error("set_range exceeds Excel limits"));
    }

    rows.iter()
        .enumerate()
        .flat_map(|(row_offset, row)| {
            row.as_array()
                .expect("validated rectangular matrix")
                .iter()
                .enumerate()
                .map(move |(col_offset, value)| (row_offset, col_offset, value))
        })
        .map(|(row_offset, col_offset, value)| {
            let row = start.row + row_offset as u32;
            let col = start.col + col_offset as u32;
            let address = format_address(CellAddress { row, col });
            let resolved = resolve_by_sheet_address(workbook, &sheet.name, &address)?;
            let editable = parse_editable_cell(value)?;
            let op = match editable {
                EditableCell::Value(value) => XlsxEditOp::SetCellValue {
                    sheet: resolved.sheet.clone(),
                    address: resolved.address.clone(),
                    element_id: resolved.element_id.clone(),
                    value,
                },
                EditableCell::Formula(formula) => XlsxEditOp::SetCellFormula {
                    sheet: resolved.sheet.clone(),
                    address: resolved.address.clone(),
                    element_id: resolved.element_id.clone(),
                    formula: normalize_formula(&formula),
                },
            };
            Ok(ParsedOperation { op, resolved })
        })
        .collect()
}

fn parse_cell_target(
    workbook: &WorkbookModel,
    payload: &Value,
) -> Result<(ResolvedCell, EditableValue)> {
    let resolved = resolve_target(workbook, payload)?;
    let value = parse_editable_value(
        payload
            .get("value")
            .ok_or_else(|| format_error("set_cell_value requires a `value` field"))?,
    )?;
    Ok((resolved, value))
}

fn parse_formula_target(
    workbook: &WorkbookModel,
    payload: &Value,
) -> Result<(ResolvedCell, String)> {
    let resolved = resolve_target(workbook, payload)?;
    let formula = payload
        .get("formula")
        .and_then(Value::as_str)
        .ok_or_else(|| format_error("set_cell_formula requires a string `formula` field"))?
        .trim()
        .to_owned();
    if formula.is_empty() {
        return Err(format_error(
            "set_cell_formula requires a non-empty formula",
        ));
    }
    Ok((resolved, formula))
}

fn resolve_target(workbook: &WorkbookModel, payload: &Value) -> Result<ResolvedCell> {
    if let Some(element_id) = payload
        .get("element_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return resolve_by_element_id(workbook, element_id);
    }

    let sheet_name = payload
        .get("sheet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format_error("edit operation requires `sheet` or `element_id`"))?;
    let address = payload
        .get("address")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format_error("edit operation requires `address` when using `sheet`"))?;

    resolve_by_sheet_address(workbook, sheet_name, address)
}

fn resolve_by_element_id(workbook: &WorkbookModel, element_id: &str) -> Result<ResolvedCell> {
    for sheet in &workbook.sheets {
        if let Some(cell) = sheet
            .cells
            .iter()
            .find(|cell| cell.element_id == element_id)
        {
            return Ok(resolved_cell(sheet, cell));
        }
    }

    Err(format_error(format!(
        "unknown cell element_id `{element_id}`"
    )))
}

fn resolve_by_sheet_address(
    workbook: &WorkbookModel,
    sheet_name: &str,
    address: &str,
) -> Result<ResolvedCell> {
    let sheet = find_sheet(workbook, sheet_name)?;
    let parsed = parse_address(address)?;
    let normalized_address = format_address(parsed);

    if let Some(cell) = sheet.cells.iter().find(|cell| {
        cell.address.eq_ignore_ascii_case(&normalized_address)
            && cell.row == parsed.row
            && cell.col == parsed.col
    }) {
        return Ok(resolved_cell(sheet, cell));
    }

    Ok(ResolvedCell {
        sheet: sheet.name.clone(),
        address: normalized_address.clone(),
        element_id: ids::cell_id(&sheet.name, &normalized_address, MODEL_SCHEMA_VERSION),
        row: parsed.row,
        col: parsed.col,
        value: CellValue::Empty,
        formula: None,
    })
}

fn resolved_cell(sheet: &crate::model::SheetModel, cell: &CellModel) -> ResolvedCell {
    ResolvedCell {
        sheet: sheet.name.clone(),
        address: cell.address.clone(),
        element_id: cell.element_id.clone(),
        row: cell.row,
        col: cell.col,
        value: cell.value.clone(),
        formula: cell.formula.clone(),
    }
}

fn find_sheet<'a>(workbook: &'a WorkbookModel, name: &str) -> Result<&'a crate::model::SheetModel> {
    workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            let available = workbook
                .sheets
                .iter()
                .map(|sheet| sheet.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format_error(format!(
                "unknown sheet `{name}`; available sheets: {available}"
            ))
        })
}

fn parse_address(address: &str) -> Result<CellAddress> {
    selector::parse_cell_address(address).map_err(format_error)
}

fn format_address(address: CellAddress) -> String {
    format!(
        "{}{}",
        crate::model::column_name((address.col - 1) as usize),
        address.row
    )
}

fn parse_editable_value(value: &Value) -> Result<EditableValue> {
    match value {
        Value::Null => Ok(EditableValue::Blank),
        Value::Bool(value) => Ok(EditableValue::Boolean(*value)),
        Value::Number(value) => value
            .as_f64()
            .map(EditableValue::Number)
            .ok_or_else(|| format_error("numeric edit values must fit in f64")),
        Value::String(value) => Ok(EditableValue::String(value.clone())),
        _ => Err(format_error(
            "edit value must be a string, number, boolean, or null",
        )),
    }
}

fn parse_editable_cell(value: &Value) -> Result<EditableCell> {
    if let Some(object) = value.as_object() {
        let kind = object.get("kind").and_then(Value::as_str);
        return match kind {
            Some("formula") => {
                let formula = object
                    .get("value")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|formula| !formula.is_empty())
                    .ok_or_else(|| {
                        format_error("set_range formula cells require a non-empty string value")
                    })?;
                Ok(EditableCell::Formula(formula.to_owned()))
            }
            Some("value") => Ok(EditableCell::Value(parse_editable_value(
                object
                    .get("value")
                    .ok_or_else(|| format_error("set_range value cells require a `value` field"))?,
            )?)),
            Some(other) => Err(format_error(format!(
                "set_range cell kind `{other}` must be `value` or `formula`"
            ))),
            None => Err(format_error(
                "set_range object cells require a `kind` of `value` or `formula`",
            )),
        };
    }
    Ok(EditableCell::Value(parse_editable_value(value)?))
}

fn required_positive_u32(payload: &Value, field: &str) -> Result<u32> {
    payload
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            format_error(format!(
                "structural edit requires a positive integer `{field}`"
            ))
        })
}

fn required_sheet_name(payload: &Value, field: &str) -> Result<String> {
    let name = payload
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format_error(format!("sheet operation requires a string `{field}`")))?;
    validate_sheet_name(name)?;
    Ok(name.to_owned())
}

fn optional_sheet_name(payload: &Value, field: &str) -> Result<Option<String>> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            validate_sheet_name(value)?;
            Ok(Some(value.clone()))
        }
        _ => Err(format_error(format!(
            "sheet operation requires `{field}` to be a string when present"
        ))),
    }
}

fn validate_sheet_name(name: &str) -> Result<()> {
    if name.is_empty() || name.chars().count() > 31 {
        return Err(format_error(
            "Excel sheet names must contain between 1 and 31 characters",
        ));
    }
    if name.starts_with('\'') || name.ends_with('\'') {
        return Err(format_error(
            "Excel sheet names cannot begin or end with an apostrophe",
        ));
    }
    if name
        .chars()
        .any(|character| matches!(character, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
    {
        return Err(format_error(
            "Excel sheet names cannot contain []:*?/\\ characters",
        ));
    }
    Ok(())
}

fn validate_new_sheet_name(
    workbook: &WorkbookModel,
    name: &str,
    renamed_sheet: Option<&str>,
) -> Result<()> {
    if workbook.sheets.iter().any(|sheet| {
        !renamed_sheet.is_some_and(|renamed| sheet.name.eq_ignore_ascii_case(renamed))
            && sheet.name.eq_ignore_ascii_case(name)
    }) {
        return Err(format_error(format!(
            "Excel sheet name `{name}` conflicts with an existing sheet (sheet names are case-insensitive)"
        )));
    }
    Ok(())
}

fn normalize_formula(formula: &str) -> String {
    let trimmed = formula.trim();
    if trimmed.starts_with('=') {
        trimmed.to_owned()
    } else {
        format!("={trimmed}")
    }
}

fn build_semantic_diff(parsed: &[ParsedOperation]) -> Vec<SemanticChange> {
    parsed
        .iter()
        .map(|operation| match &operation.op {
            XlsxEditOp::SetCellValue { value, .. } => SemanticChange {
                target: operation.resolved.target_label(),
                element_id: operation.resolved.element_id.clone(),
                change: "value".into(),
                before: format_cell_value(&operation.resolved.value),
                after: format_editable_value(value),
            },
            XlsxEditOp::SetCellFormula { formula, .. } => SemanticChange {
                target: operation.resolved.target_label(),
                element_id: operation.resolved.element_id.clone(),
                change: "formula".into(),
                before: operation.resolved.formula.clone(),
                after: Some(formula.clone()),
            },
            XlsxEditOp::InsertRow { .. } => {
                unreachable!("structural edits are validated separately")
            }
            XlsxEditOp::DeleteRow { .. } => {
                unreachable!("structural edits are validated separately")
            }
            XlsxEditOp::InsertColumn { .. } | XlsxEditOp::DeleteColumn { .. } => {
                unreachable!("structural edits are validated separately")
            }
            XlsxEditOp::SetRange { .. } => {
                unreachable!("set_range is expanded into cell edits during validation")
            }
            XlsxEditOp::AddSheet { .. }
            | XlsxEditOp::RenameSheet { .. }
            | XlsxEditOp::DeleteSheet { .. }
            | XlsxEditOp::MergeCells { .. }
            | XlsxEditOp::UnmergeCells { .. }
            | XlsxEditOp::SetColumnWidth { .. }
            | XlsxEditOp::SetRowHeight { .. }
            | XlsxEditOp::FreezePanes { .. }
            | XlsxEditOp::DefineName { .. }
            | XlsxEditOp::DeleteName { .. }
            | XlsxEditOp::HideSheet { .. } | XlsxEditOp::SetTabColor { .. }
            | XlsxEditOp::SetAutoFilter { .. }
            | XlsxEditOp::SetPrintArea { .. }
            | XlsxEditOp::SetPrintTitles { .. }
            | XlsxEditOp::SetPageOrientation { .. }
            | XlsxEditOp::SetPaperSize { .. }
                | XlsxEditOp::SetPrintScale { .. }
 | XlsxEditOp::SetFitToPage { .. }
        | XlsxEditOp::SetCenterOnPage { .. }
        | XlsxEditOp::SetPageMargins { .. }
        | XlsxEditOp::SetHeaderFooter { .. }
        | XlsxEditOp::SetCellFont { .. }
        | XlsxEditOp::SetCellFill { .. }
        | XlsxEditOp::SetSheetZoom { .. }
        | XlsxEditOp::SetShowGridlines { .. }
        | XlsxEditOp::SetRightToLeft { .. }
        | XlsxEditOp::InsertComment { .. }
                | XlsxEditOp::InsertPicture { .. } => {
                unreachable!(
                    "sheet, merge, dimension, freeze, and define_name edits are validated separately"
                )
            }
        })
        .collect()
}

fn build_dependency_impact(
    parsed: &[ParsedOperation],
    graph: &DependencyGraph,
    workbook: &WorkbookModel,
) -> DependencyImpact {
    let locations = cell_locations(workbook);
    let mut forward = BTreeSet::new();

    for operation in parsed {
        for edge in graph.reverse_at(&operation.resolved.sheet, &operation.resolved.address) {
            if let Some((sheet, address)) = locations.get(&edge.from_element_id) {
                forward.insert(format!("{sheet}!{address}"));
            }
        }
    }

    DependencyImpact {
        forward: forward.into_iter().collect(),
        notes: vec!["refs parsed; values not evaluated".into()],
    }
}

fn cell_locations(
    workbook: &WorkbookModel,
) -> std::collections::BTreeMap<String, (String, String)> {
    workbook
        .sheets
        .iter()
        .flat_map(|sheet| {
            sheet.cells.iter().map(|cell| {
                (
                    cell.element_id.clone(),
                    (sheet.name.clone(), cell.address.clone()),
                )
            })
        })
        .collect()
}

fn editable_value_to_json(value: &EditableValue) -> Value {
    match value {
        EditableValue::Blank => Value::Null,
        EditableValue::Boolean(value) => Value::Bool(*value),
        EditableValue::Number(value) => {
            if value.fract() == 0.0 && value.abs() <= i64::MAX as f64 {
                Value::Number(serde_json::Number::from(*value as i64))
            } else {
                serde_json::Number::from_f64(*value)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            }
        }
        EditableValue::String(value) => Value::String(value.clone()),
    }
}

fn operation_to_semantic(op: &XlsxEditOp) -> SemanticOperation {
    match op {
        XlsxEditOp::SetCellValue {
            sheet,
            address,
            element_id,
            value,
        } => SemanticOperation {
            kind: "set_cell_value".into(),
            payload: serde_json::json!({
                "sheet": sheet,
                "address": address,
                "element_id": element_id,
                "value": editable_value_to_json(value),
            }),
        },
        XlsxEditOp::SetCellFormula {
            sheet,
            address,
            element_id,
            formula,
        } => SemanticOperation {
            kind: "set_cell_formula".into(),
            payload: serde_json::json!({
                "sheet": sheet,
                "address": address,
                "element_id": element_id,
                "formula": formula,
            }),
        },
        XlsxEditOp::InsertRow { .. } => unreachable!("structural edits are validated separately"),
        XlsxEditOp::DeleteRow { .. } => unreachable!("structural edits are validated separately"),
        XlsxEditOp::InsertColumn { .. } | XlsxEditOp::DeleteColumn { .. } => {
            unreachable!("structural edits are validated separately")
        }
        XlsxEditOp::SetRange { .. } => {
            unreachable!("set_range is expanded into cell edits during validation")
        }
        XlsxEditOp::AddSheet { .. }
        | XlsxEditOp::RenameSheet { .. }
        | XlsxEditOp::DeleteSheet { .. }
        | XlsxEditOp::MergeCells { .. }
        | XlsxEditOp::UnmergeCells { .. }
        | XlsxEditOp::SetColumnWidth { .. }
        | XlsxEditOp::SetRowHeight { .. }
        | XlsxEditOp::FreezePanes { .. }
        | XlsxEditOp::DefineName { .. }
        | XlsxEditOp::DeleteName { .. }
        | XlsxEditOp::HideSheet { .. }
        | XlsxEditOp::SetTabColor { .. }
        | XlsxEditOp::SetAutoFilter { .. }
        | XlsxEditOp::SetPrintArea { .. }
        | XlsxEditOp::SetPrintTitles { .. }
        | XlsxEditOp::SetPageOrientation { .. }
        | XlsxEditOp::SetPaperSize { .. }
        | XlsxEditOp::SetPrintScale { .. }
        | XlsxEditOp::SetFitToPage { .. }
        | XlsxEditOp::SetCenterOnPage { .. }
        | XlsxEditOp::SetPageMargins { .. }
        | XlsxEditOp::SetHeaderFooter { .. }
        | XlsxEditOp::SetCellFont { .. }
        | XlsxEditOp::SetCellFill { .. }
        | XlsxEditOp::SetSheetZoom { .. }
        | XlsxEditOp::SetShowGridlines { .. }
        | XlsxEditOp::SetRightToLeft { .. }
        | XlsxEditOp::InsertComment { .. }
        | XlsxEditOp::InsertPicture { .. } => {
            unreachable!(
                "sheet, merge, dimension, freeze, define_name, insert_comment, and insert_picture edits are validated separately"
            )
        }
    }
}

fn format_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: PathBuf::from("<edit validation>"),
        message: message.into(),
    }
}

impl ResolvedCell {
    fn target_label(&self) -> String {
        format!("{}!{}", self.sheet, self.address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CellModel, NamedRange, PreservationStatus, SheetDimensions, SheetModel, UnmodeledMap,
    };

    fn workbook_fixture() -> WorkbookModel {
        WorkbookModel {
            workbook_id: "wb_fixture".into(),
            sheets: vec![
                fixture_sheet(
                    "Inputs",
                    vec![
                        fixture_cell("c_inputs_a1", "A1", None),
                        fixture_cell("c_inputs_b1", "B1", None),
                    ],
                ),
                fixture_sheet(
                    "Summary",
                    vec![
                        fixture_cell("c_summary_b2", "B2", Some("=Inputs!A1")),
                        fixture_cell("c_summary_c2", "C2", Some("=SUM(Inputs!A1:A10)")),
                        fixture_cell("c_summary_d2", "D2", Some("=TaxRate * C2")),
                    ],
                ),
            ],
            named_ranges: vec![NamedRange {
                element_id: "nr_tax_rate".into(),
                name: "TaxRate".into(),
                formula: "=Inputs!$B$1".into(),
            }],
            style_table: Vec::new(),
            comments: Vec::new(),
            charts: Vec::new(),
            pictures: Vec::new(),
            unmodeled: UnmodeledMap {
                charts: PreservationStatus::Preserved,
                pivots: PreservationStatus::Preserved,
                vba: PreservationStatus::Preserved,
                other_ooxml_parts: PreservationStatus::Preserved,
            },
        }
    }

    fn fixture_sheet(name: &str, cells: Vec<CellModel>) -> SheetModel {
        SheetModel {
            element_id: format!("sh_{name}"),
            name: name.into(),
            index: 0,
            dimensions: SheetDimensions { rows: 10, cols: 4 },
            merges: Vec::new(),
            freeze_panes: None,
            zoom: None,
            show_gridlines: true,
            right_to_left: false,
            tab_color: None,
            auto_filter: None,
            print_area: None,
            print_titles: None,
            page_orientation: None,
            paper_size: None,
            print_scale: None,
            fit_to_page: None,
            center_on_page: None,
            page_margins: None,
            header_footer: None,
            cells,
        }
    }

    fn fixture_cell(element_id: &str, address: &str, formula: Option<&str>) -> CellModel {
        let split = address
            .find(|character: char| character.is_ascii_digit())
            .expect("address");
        let (column, row) = address.split_at(split);
        CellModel {
            element_id: element_id.into(),
            address: address.into(),
            row: row.parse().expect("row"),
            col: u32::from(column.as_bytes()[0] - b'A' + 1),
            value: CellValue::Empty,
            formula: formula.map(str::to_owned),
            style_id: None,
            number_format: None,
        }
    }

    fn envelope(model: WorkbookModel) -> ArtifactEnvelope {
        ArtifactEnvelope {
            format_id: FORMAT_ID.into(),
            schema_id: MODEL_SCHEMA_ID.into(),
            schema_version: MODEL_SCHEMA_VERSION,
            payload: serde_json::to_value(model).expect("serialize workbook"),
        }
    }

    #[test]
    fn validated_operations_round_trip_through_revalidation() {
        let envelope = envelope(workbook_fixture());
        let validated = validate(
            &envelope,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A1",
                    "value": 42
                }),
            }],
        )
        .expect("valid edit");

        assert_eq!(
            validated.operations[0].payload["value"],
            serde_json::json!(42)
        );

        let revalidated = validate(&envelope, &validated.operations).expect("round-trip edit");
        assert_eq!(revalidated.operations, validated.operations);
        assert_eq!(revalidated.semantic_diff, validated.semantic_diff);
    }

    #[test]
    fn sparse_cell_edit_finds_range_dependents() {
        let validated = validate(
            &envelope(workbook_fixture()),
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A5",
                    "value": 10
                }),
            }],
        )
        .expect("valid edit");

        assert_eq!(validated.semantic_diff[0].target, "Inputs!A5");
        assert_eq!(
            validated.dependency_impact.forward,
            vec!["Summary!C2".to_string()]
        );
    }

    #[test]
    fn validates_set_cell_formula_with_normalized_formula_and_dependency_impact() {
        let validated = validate(
            &envelope(workbook_fixture()),
            &[SemanticOperation {
                kind: "set_cell_formula".into(),
                payload: serde_json::json!({
                    "sheet": "Summary",
                    "address": "B2",
                    "formula": "Inputs!A1*1.1"
                }),
            }],
        )
        .expect("valid edit");

        assert_eq!(validated.schema_id, SCHEMA_ID);
        assert_eq!(validated.semantic_diff.len(), 1);
        assert_eq!(validated.semantic_diff[0].change, "formula");
        assert_eq!(
            validated.semantic_diff[0].before.as_deref(),
            Some("=Inputs!A1")
        );
        assert_eq!(
            validated.semantic_diff[0].after.as_deref(),
            Some("=Inputs!A1*1.1")
        );
        assert_eq!(validated.dependency_impact.forward, Vec::<String>::new());
        assert_eq!(
            validated.dependency_impact.notes,
            vec!["refs parsed; values not evaluated".to_string()]
        );
    }

    #[test]
    fn validates_set_cell_value_with_reverse_dependents() {
        let validated = validate(
            &envelope(workbook_fixture()),
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A1",
                    "value": 42
                }),
            }],
        )
        .expect("valid edit");

        assert_eq!(validated.semantic_diff[0].change, "value");
        assert_eq!(validated.semantic_diff[0].before, None);
        assert_eq!(validated.semantic_diff[0].after.as_deref(), Some("42"));
        assert_eq!(
            validated.dependency_impact.forward,
            vec!["Summary!B2".to_string(), "Summary!C2".to_string()]
        );
    }

    #[test]
    fn validates_set_cell_formula_by_element_id() {
        let validated = validate(
            &envelope(workbook_fixture()),
            &[SemanticOperation {
                kind: "set_cell_formula".into(),
                payload: serde_json::json!({
                    "element_id": "c_summary_b2",
                    "formula": "=Inputs!A1+1"
                }),
            }],
        )
        .expect("valid edit");

        assert_eq!(validated.semantic_diff[0].target, "Summary!B2");
        assert_eq!(
            validated.semantic_diff[0].after.as_deref(),
            Some("=Inputs!A1+1")
        );
    }

    #[test]
    fn rejects_unknown_sheet_with_available_names() {
        let error = validate(
            &envelope(workbook_fixture()),
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Missing",
                    "address": "A1",
                    "value": 1
                }),
            }],
        )
        .expect_err("unknown sheet");

        match error {
            DotallError::Format { message, .. } => {
                assert!(message.contains("unknown sheet `Missing`"));
                assert!(message.contains("Inputs"));
                assert!(message.contains("Summary"));
            }
            other => panic!("expected format error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_cell_targets() {
        let error = validate(
            &envelope(workbook_fixture()),
            &[
                SemanticOperation {
                    kind: "set_cell_value".into(),
                    payload: serde_json::json!({
                        "sheet": "Inputs",
                        "address": "A1",
                        "value": 1
                    }),
                },
                SemanticOperation {
                    kind: "set_cell_formula".into(),
                    payload: serde_json::json!({
                        "element_id": "c_inputs_a1",
                        "formula": "=1"
                    }),
                },
            ],
        )
        .expect_err("duplicate target");

        match error {
            DotallError::Format { message, .. } => {
                assert!(message.contains("multiple operations target the same cell"));
            }
            other => panic!("expected format error, got {other:?}"),
        }
    }
}
