pub mod impact;
mod ops;
pub mod transform;
mod validate;
pub(crate) mod writer;

pub use ops::{
    EditableCell, EditableValue, SCHEMA_ID, SCHEMA_VERSION, XlsxEditOp, format_cell_value,
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
            "set_range" => Err(invalid_operation(
                "validated set_range operations must be expanded into cell edits",
            )),
            kind => Err(invalid_operation(format!(
                "unsupported validated edit operation `{kind}`"
            ))),
        })
        .collect()
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

fn invalid_operation(message: impl Into<String>) -> dotall_core::DotallError {
    dotall_core::DotallError::Format {
        format_id: crate::FORMAT_ID.into(),
        path: "<validated XLSX edit>".into(),
        message: message.into(),
    }
}
