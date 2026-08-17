pub mod impact;
mod ops;
pub mod transform;
mod validate;
pub(crate) mod writer;

pub use ops::{
    DeleteSheetPolicy, EditableCell, EditableValue, SCHEMA_ID, SCHEMA_VERSION, XlsxEditOp,
    format_cell_value,
};
pub use validate::{validate, validate_with_source};

pub(crate) fn parse_validated_operations(
    operations: &[dotall_core::SemanticOperation],
) -> dotall_core::Result<Vec<XlsxEditOp>> {
    operations
        .iter()
        .map(|operation| match operation.kind.as_str() {
            "set_cell_value" => Ok(XlsxEditOp::SetCellValue {
                sheet: required_string(operation, "sheet")?,
                address: required_string(operation, "address")?,
                element_id: required_string(operation, "element_id")?,
                value: match operation.payload.get("value") {
                    Some(serde_json::Value::Null) => EditableValue::Blank,
                    Some(serde_json::Value::Bool(value)) => EditableValue::Boolean(*value),
                    Some(serde_json::Value::Number(value)) => value
                        .as_f64()
                        .map(EditableValue::Number)
                        .ok_or_else(|| invalid_operation("numeric edit values must fit in f64"))?,
                    Some(serde_json::Value::String(value)) => EditableValue::String(value.clone()),
                    _ => {
                        return Err(invalid_operation(
                            "set_cell_value requires a string, number, boolean, or null value",
                        ));
                    }
                },
            }),
            "set_cell_formula" => Ok(XlsxEditOp::SetCellFormula {
                sheet: required_string(operation, "sheet")?,
                address: required_string(operation, "address")?,
                element_id: required_string(operation, "element_id")?,
                formula: required_string(operation, "formula")?,
            }),
            "insert_row" => Ok(XlsxEditOp::InsertRow {
                sheet: required_string(operation, "sheet")?,
                at: required_positive_u32(operation, "at")?,
                count: required_positive_u32(operation, "count")?,
            }),
            "delete_row" => Ok(XlsxEditOp::DeleteRow {
                sheet: required_string(operation, "sheet")?,
                at: required_positive_u32(operation, "at")?,
                count: required_positive_u32(operation, "count")?,
            }),
            "insert_column" => Ok(XlsxEditOp::InsertColumn {
                sheet: required_string(operation, "sheet")?,
                at: required_positive_u32(operation, "at")?,
                count: required_positive_u32(operation, "count")?,
            }),
            "delete_column" => Ok(XlsxEditOp::DeleteColumn {
                sheet: required_string(operation, "sheet")?,
                at: required_positive_u32(operation, "at")?,
                count: required_positive_u32(operation, "count")?,
            }),
            "add_sheet" => Ok(XlsxEditOp::AddSheet {
                name: required_string(operation, "name")?,
                after: optional_string(operation, "after")?,
            }),
            "rename_sheet" => Ok(XlsxEditOp::RenameSheet {
                from: required_string(operation, "from")?,
                to: required_string(operation, "to")?,
            }),
            "delete_sheet" => Ok(XlsxEditOp::DeleteSheet {
                name: required_string(operation, "name")?,
                dependency_policy: match operation
                    .payload
                    .get("dependency_policy")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("reject_if_referenced")
                {
                    "reject_if_referenced" => DeleteSheetPolicy::RejectIfReferenced,
                    "replace_references_with_ref_error" => {
                        DeleteSheetPolicy::ReplaceReferencesWithRefError
                    }
                    value => {
                        return Err(invalid_operation(format!(
                            "unsupported delete_sheet dependency_policy `{value}`"
                        )));
                    }
                },
            }),
            "merge_cells" => Ok(XlsxEditOp::MergeCells {
                sheet: required_string(operation, "sheet")?,
                range: required_string(operation, "range")?,
            }),
            "unmerge_cells" => Ok(XlsxEditOp::UnmergeCells {
                sheet: required_string(operation, "sheet")?,
                range: required_string(operation, "range")?,
            }),
            "set_column_width" => Ok(XlsxEditOp::SetColumnWidth {
                sheet: required_string(operation, "sheet")?,
                column: required_string(operation, "column")?,
                width: required_positive_f64(operation, "width")?,
            }),
            "set_row_height" => Ok(XlsxEditOp::SetRowHeight {
                sheet: required_string(operation, "sheet")?,
                row: required_positive_u32(operation, "row")?,
                height: required_positive_f64(operation, "height")?,
            }),
            "freeze_panes" => Ok(XlsxEditOp::FreezePanes {
                sheet: required_string(operation, "sheet")?,
                cell: optional_string(operation, "cell")?,
            }),
            "define_name" => Ok(XlsxEditOp::DefineName {
                name: required_string(operation, "name")?,
                formula: required_string(operation, "formula")?,
            }),
            "delete_name" => Ok(XlsxEditOp::DeleteName {
                name: required_string(operation, "name")?,
            }),
            "hide_sheet" => Ok(XlsxEditOp::HideSheet {
                sheet: required_string(operation, "sheet")?,
                hidden: required_bool(operation, "hidden")?,
            }),
            "set_tab_color" => Ok(XlsxEditOp::SetTabColor {
                sheet: required_string(operation, "sheet")?,
                color: optional_string(operation, "color")?,
            }),
            "set_auto_filter" => Ok(XlsxEditOp::SetAutoFilter {
                sheet: required_string(operation, "sheet")?,
                range: optional_string(operation, "range")?,
            }),
            "set_print_area" => Ok(XlsxEditOp::SetPrintArea {
                sheet: required_string(operation, "sheet")?,
                range: optional_string(operation, "range")?,
            }),
            "set_print_titles" => Ok(XlsxEditOp::SetPrintTitles {
                sheet: required_string(operation, "sheet")?,
                rows: optional_string(operation, "rows")?,
                cols: optional_string(operation, "cols")?,
            }),
            "set_range" => Err(invalid_operation(
                "validated set_range operations must be expanded into cell edits",
            )),
            kind => Err(invalid_operation(format!(
                "unsupported validated edit operation `{kind}`"
            ))),
        })
        .collect()
}

fn required_bool(
    operation: &dotall_core::SemanticOperation,
    field: &str,
) -> dotall_core::Result<bool> {
    operation
        .payload
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| {
            invalid_operation(format!(
                "validated edit operation requires boolean `{field}`"
            ))
        })
}

fn required_positive_u32(
    operation: &dotall_core::SemanticOperation,
    field: &str,
) -> dotall_core::Result<u32> {
    operation
        .payload
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            invalid_operation(format!(
                "validated edit operation requires positive `{field}`"
            ))
        })
}

fn required_positive_f64(
    operation: &dotall_core::SemanticOperation,
    field: &str,
) -> dotall_core::Result<f64> {
    operation
        .payload
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| {
            invalid_operation(format!(
                "validated edit operation requires positive `{field}`"
            ))
        })
}

fn required_string(
    operation: &dotall_core::SemanticOperation,
    field: &str,
) -> dotall_core::Result<String> {
    operation
        .payload
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid_operation(format!("validated edit operation requires `{field}`")))
}

fn optional_string(
    operation: &dotall_core::SemanticOperation,
    field: &str,
) -> dotall_core::Result<Option<String>> {
    match operation.payload.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        _ => Err(invalid_operation(format!(
            "validated edit operation requires `{field}` to be a non-empty string when present"
        ))),
    }
}

fn invalid_operation(message: impl Into<String>) -> dotall_core::DotallError {
    dotall_core::DotallError::Format {
        format_id: crate::FORMAT_ID.into(),
        path: "<validated XLSX edit>".into(),
        message: message.into(),
    }
}
