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
    if operations.iter().any(|operation| {
        matches!(
            operation.kind.as_str(),
            "add_sheet" | "rename_sheet" | "delete_sheet"
        )
    }) {
        return validate_sheet_operation(source, model, operations);
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
            | XlsxEditOp::DeleteSheet { .. } => {
                unreachable!("sheet edits are validated separately")
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
        | XlsxEditOp::DeleteSheet { .. } => {
            unreachable!("sheet edits are validated separately")
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
