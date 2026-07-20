mod ops;
mod validate;
pub(crate) mod writer;

pub use ops::{EditableValue, SCHEMA_ID, SCHEMA_VERSION, XlsxEditOp, format_cell_value};
pub use validate::validate;

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
            kind => Err(invalid_operation(format!(
                "unsupported validated edit operation `{kind}`"
            ))),
        })
        .collect()
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
