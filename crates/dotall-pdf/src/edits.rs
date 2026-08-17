use std::io::Cursor;

use dotall_core::{
    DependencyImpact, DotallError, PatchedOutput, Result, SemanticChange, SemanticOperation,
    ValidatedEdit,
};
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::FORMAT_ID;
use crate::model::{PdfDocumentModel, PdfFieldModel, SCHEMA_ID, SCHEMA_VERSION};

pub fn validate(
    model: &PdfDocumentModel,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "pdf supports a single edit operation per transaction",
        ));
    }
    if model.encrypted {
        return Err(format_error("cannot edit encrypted PDF"));
    }
    if model.fields.iter().any(|field| field.field_type == "sig") {
        return Err(format_error("cannot edit signed PDF"));
    }
    let operation = &operations[0];
    match operation.kind.as_str() {
        "set_form_field" => validate_set_form_field(model, operation),
        "clear_form_field" => validate_clear_form_field(model, operation),
        "clear_all_form_fields" => validate_clear_all_form_fields(model, operation),
        "set_form_fields" => validate_set_form_fields(model, operation),
        "set_form_field_readonly" => validate_set_form_field_readonly(model, operation),
        "set_form_field_required" => validate_set_form_field_required(model, operation),
        "set_form_field_multiline" => validate_set_form_field_multiline(model, operation),
        "set_form_field_password" => validate_set_form_field_password(model, operation),
        "set_form_field_max_length" => validate_set_form_field_max_length(model, operation),
        "set_form_field_comb" => validate_set_form_field_comb(model, operation),
        "set_form_field_do_not_scroll" => validate_set_form_field_do_not_scroll(model, operation),
        "set_form_field_do_not_spell_check" => {
            validate_set_form_field_do_not_spell_check(model, operation)
        }
        "set_form_field_rich_text" => validate_set_form_field_rich_text(model, operation),
        "set_form_field_no_export" => validate_set_form_field_no_export(model, operation),
        "set_form_field_multi_select" => validate_set_form_field_multi_select(model, operation),
        "set_document_metadata" => validate_set_document_metadata(model, operation),
        "clear_document_metadata" => validate_clear_document_metadata(model, operation),
        other => Err(format_error(format!(
            "unsupported pdf edit `{other}`; use set_form_field, clear_form_field, clear_all_form_fields, set_form_fields, set_form_field_readonly, set_form_field_required, set_form_field_multiline, set_form_field_password, set_form_field_max_length, set_form_field_comb, set_form_field_do_not_scroll, set_form_field_do_not_spell_check, set_form_field_rich_text, set_form_field_no_export, set_form_field_multi_select, set_document_metadata, or clear_document_metadata"
        ))),
    }
}

fn validate_set_form_field(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let value = required_str(&operation.payload, "value")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.read_only {
        return Err(format_error("field is read-only"));
    }
    let value = match field.field_type.as_str() {
        "tx" | "ch" => value.to_owned(),
        "btn" => resolve_btn_value(field, value)?,
        other => {
            return Err(format_error(format!(
                "field type `{other}` is not supported in v0"
            )));
        }
    };
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field".into(),
            payload: serde_json::json!({
                "name": field.name,
                "value": value,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: field.name.clone(),
            element_id: field.element_id.clone(),
            change: "set_form_field".into(),
            before: Some(field.value.clone()),
            after: Some(value),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_clear_form_field(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.read_only {
        return Err(format_error("field is read-only"));
    }
    let value = clear_value_for_field(field)?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "clear_form_field".into(),
            payload: serde_json::json!({
                "name": field.name,
                "value": value,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: field.name.clone(),
            element_id: field.element_id.clone(),
            change: "clear_form_field".into(),
            before: Some(field.value.clone()),
            after: Some(value),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn clear_value_for_field(field: &PdfFieldModel) -> Result<String> {
    match field.field_type.as_str() {
        "tx" | "ch" => Ok(String::new()),
        "btn" => {
            if field
                .export_values
                .iter()
                .any(|value| value.eq_ignore_ascii_case("Off"))
            {
                Ok("Off".into())
            } else {
                resolve_btn_value(field, "Off")
            }
        }
        other => Err(format_error(format!(
            "field type `{other}` is not supported in v0"
        ))),
    }
}

fn validate_clear_all_form_fields(
    model: &PdfDocumentModel,
    _operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let mut fields = Vec::new();
    let mut before_parts = Vec::new();
    let mut after_parts = Vec::new();
    for field in &model.fields {
        if field.read_only {
            continue;
        }
        let value = clear_value_for_field(field)?;
        before_parts.push(format!("{}={}", field.name, field.value));
        after_parts.push(format!("{}={value}", field.name));
        fields.push(serde_json::json!({
            "name": field.name,
            "value": value,
            "element_id": field.element_id,
            "field_type": field.field_type,
        }));
    }
    if fields.is_empty() {
        return Err(format_error(
            "clear_all_form_fields found no editable form fields",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "clear_all_form_fields".into(),
            payload: serde_json::json!({ "fields": fields }),
        }],
        semantic_diff: vec![SemanticChange {
            target: "document:/AcroForm".into(),
            element_id: model.document_id.clone(),
            change: "clear_all_form_fields".into(),
            before: Some(before_parts.join("; ")),
            after: Some(after_parts.join("; ")),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_fields(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let fields_map = operation
        .payload
        .get("fields")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| format_error("set_form_fields requires a `fields` object"))?;
    if fields_map.is_empty() {
        return Err(format_error("set_form_fields requires at least one field"));
    }

    let mut fields = Vec::new();
    let mut before_parts = Vec::new();
    let mut after_parts = Vec::new();
    for (name, raw_value) in fields_map {
        let value = raw_value.as_str().ok_or_else(|| {
            format_error(format!(
                "set_form_fields value for `{name}` must be a string"
            ))
        })?;
        let field = model
            .fields
            .iter()
            .find(|field| field.name == *name)
            .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
        if field.read_only {
            return Err(format_error(format!("field `{name}` is read-only")));
        }
        let resolved = match field.field_type.as_str() {
            "tx" | "ch" => value.to_owned(),
            "btn" => resolve_btn_value(field, value)?,
            other => {
                return Err(format_error(format!(
                    "field type `{other}` is not supported in v0"
                )));
            }
        };
        before_parts.push(format!("{}={}", field.name, field.value));
        after_parts.push(format!("{}={resolved}", field.name));
        fields.push(serde_json::json!({
            "name": field.name,
            "value": resolved,
            "element_id": field.element_id,
            "field_type": field.field_type,
        }));
    }

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_fields".into(),
            payload: serde_json::json!({ "fields": fields }),
        }],
        semantic_diff: vec![SemanticChange {
            target: "document:/AcroForm".into(),
            element_id: model.document_id.clone(),
            change: "set_form_fields".into(),
            before: Some(before_parts.join("; ")),
            after: Some(after_parts.join("; ")),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_readonly(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let readonly = required_bool(&operation.payload, "readonly")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type == "sig" {
        return Err(format_error("cannot edit signature fields"));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_readonly".into(),
            payload: serde_json::json!({
                "name": field.name,
                "readonly": readonly,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_readonly".into(),
            before: Some(if field.read_only { "true" } else { "false" }.into()),
            after: Some(if readonly { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_required(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let required = required_bool(&operation.payload, "required")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type == "sig" {
        return Err(format_error("cannot edit signature fields"));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_required".into(),
            payload: serde_json::json!({
                "name": field.name,
                "required": required,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_required".into(),
            before: Some(if field.required { "true" } else { "false" }.into()),
            after: Some(if required { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_multiline(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let multiline = required_bool(&operation.payload, "multiline")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type != "tx" {
        return Err(format_error(
            "set_form_field_multiline only applies to text (tx) fields",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_multiline".into(),
            payload: serde_json::json!({
                "name": field.name,
                "multiline": multiline,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_multiline".into(),
            before: Some(if field.multiline { "true" } else { "false" }.into()),
            after: Some(if multiline { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_password(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let password = required_bool(&operation.payload, "password")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type != "tx" {
        return Err(format_error(
            "set_form_field_password only applies to text (tx) fields",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_password".into(),
            payload: serde_json::json!({
                "name": field.name,
                "password": password,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_password".into(),
            before: Some(if field.password { "true" } else { "false" }.into()),
            after: Some(if password { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_max_length(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let max_length = optional_positive_u32(&operation.payload, "max_length")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type != "tx" {
        return Err(format_error(
            "set_form_field_max_length only applies to text (tx) fields",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_max_length".into(),
            payload: serde_json::json!({
                "name": field.name,
                "max_length": max_length,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_max_length".into(),
            before: Some(match field.max_length {
                Some(value) => value.to_string(),
                None => "cleared".into(),
            }),
            after: Some(match max_length {
                Some(value) => value.to_string(),
                None => "cleared".into(),
            }),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_comb(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let comb = required_bool(&operation.payload, "comb")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type != "tx" {
        return Err(format_error(
            "set_form_field_comb only applies to text (tx) fields",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_comb".into(),
            payload: serde_json::json!({
                "name": field.name,
                "comb": comb,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_comb".into(),
            before: Some(if field.comb { "true" } else { "false" }.into()),
            after: Some(if comb { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_do_not_scroll(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let do_not_scroll = required_bool(&operation.payload, "do_not_scroll")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type != "tx" {
        return Err(format_error(
            "set_form_field_do_not_scroll only applies to text (tx) fields",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_do_not_scroll".into(),
            payload: serde_json::json!({
                "name": field.name,
                "do_not_scroll": do_not_scroll,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_do_not_scroll".into(),
            before: Some(if field.do_not_scroll { "true" } else { "false" }.into()),
            after: Some(if do_not_scroll { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_do_not_spell_check(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let do_not_spell_check = required_bool(&operation.payload, "do_not_spell_check")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type != "tx" {
        return Err(format_error(
            "set_form_field_do_not_spell_check only applies to text (tx) fields",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_do_not_spell_check".into(),
            payload: serde_json::json!({
                "name": field.name,
                "do_not_spell_check": do_not_spell_check,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_do_not_spell_check".into(),
            before: Some(
                if field.do_not_spell_check {
                    "true"
                } else {
                    "false"
                }
                .into(),
            ),
            after: Some(if do_not_spell_check { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_no_export(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let no_export = required_bool(&operation.payload, "no_export")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type == "sig" {
        return Err(format_error("cannot edit signature fields"));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_no_export".into(),
            payload: serde_json::json!({
                "name": field.name,
                "no_export": no_export,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_no_export".into(),
            before: Some(if field.no_export { "true" } else { "false" }.into()),
            after: Some(if no_export { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_multi_select(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let multi_select = required_bool(&operation.payload, "multi_select")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type != "ch" {
        return Err(format_error(
            "set_form_field_multi_select only applies to choice (ch) fields",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_multi_select".into(),
            payload: serde_json::json!({
                "name": field.name,
                "multi_select": multi_select,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_multi_select".into(),
            before: Some(if field.multi_select { "true" } else { "false" }.into()),
            after: Some(if multi_select { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_rich_text(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let rich_text = required_bool(&operation.payload, "rich_text")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type != "tx" {
        return Err(format_error(
            "set_form_field_rich_text only applies to text (tx) fields",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_rich_text".into(),
            payload: serde_json::json!({
                "name": field.name,
                "rich_text": rich_text,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_rich_text".into(),
            before: Some(if field.rich_text { "true" } else { "false" }.into()),
            after: Some(if rich_text { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_document_metadata(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let title = optional_str(&operation.payload, "title")?;
    let author = optional_str(&operation.payload, "author")?;
    let subject = optional_str(&operation.payload, "subject")?;
    if title.is_none() && author.is_none() && subject.is_none() {
        return Err(format_error(
            "set_document_metadata requires at least one of title, author, or subject",
        ));
    }

    let mut payload = serde_json::Map::new();
    let mut before_parts = Vec::new();
    let mut after_parts = Vec::new();
    if let Some(title) = title {
        payload.insert("title".into(), serde_json::Value::String(title.to_owned()));
        before_parts.push(format!(
            "title={}",
            model.metadata.title.as_deref().unwrap_or("")
        ));
        after_parts.push(format!("title={title}"));
    }
    if let Some(author) = author {
        payload.insert(
            "author".into(),
            serde_json::Value::String(author.to_owned()),
        );
        before_parts.push(format!(
            "author={}",
            model.metadata.author.as_deref().unwrap_or("")
        ));
        after_parts.push(format!("author={author}"));
    }
    if let Some(subject) = subject {
        payload.insert(
            "subject".into(),
            serde_json::Value::String(subject.to_owned()),
        );
        before_parts.push(format!(
            "subject={}",
            model.metadata.subject.as_deref().unwrap_or("")
        ));
        after_parts.push(format!("subject={subject}"));
    }

    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_document_metadata".into(),
            payload: serde_json::Value::Object(payload),
        }],
        semantic_diff: vec![SemanticChange {
            target: "document:/Info".into(),
            element_id: model.document_id.clone(),
            change: "set_document_metadata".into(),
            before: Some(before_parts.join("; ")),
            after: Some(after_parts.join("; ")),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_clear_document_metadata(
    model: &PdfDocumentModel,
    _operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let before = format!(
        "title={}; author={}; subject={}",
        model.metadata.title.as_deref().unwrap_or(""),
        model.metadata.author.as_deref().unwrap_or(""),
        model.metadata.subject.as_deref().unwrap_or("")
    );
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "clear_document_metadata".into(),
            payload: serde_json::json!({}),
        }],
        semantic_diff: vec![SemanticChange {
            target: "document:/Info".into(),
            element_id: model.document_id.clone(),
            change: "clear_document_metadata".into(),
            before: Some(before),
            after: Some("title=; author=; subject=".into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

pub fn apply(source: &std::path::Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
    let bytes = std::fs::read(source).map_err(|error| DotallError::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    apply_bytes(&bytes, edit)
}

pub fn apply_bytes(package: &[u8], edit: &ValidatedEdit) -> Result<PatchedOutput> {
    let operation = edit
        .operations
        .first()
        .ok_or_else(|| format_error("validated edit is missing operations"))?;
    let mut document = Document::load_mem(package)
        .map_err(|error| format_error(format!("invalid PDF: {error}")))?;
    match operation.kind.as_str() {
        "set_form_field" | "clear_form_field" => {
            let name = required_str(&operation.payload, "name")?;
            let value = required_str(&operation.payload, "value")?;
            let field_type = operation
                .payload
                .get("field_type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tx");
            set_field_value(&mut document, name, value, field_type)?;
            set_need_appearances(&mut document)?;
        }
        "clear_all_form_fields" | "set_form_fields" => {
            let fields = operation
                .payload
                .get("fields")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| format_error(format!("{} requires `fields`", operation.kind)))?;
            for field in fields {
                let name = field
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        format_error(format!("{} field missing name", operation.kind))
                    })?;
                let value = field
                    .get("value")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        format_error(format!("{} field missing value", operation.kind))
                    })?;
                let field_type = field
                    .get("field_type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("tx");
                set_field_value(&mut document, name, value, field_type)?;
            }
            set_need_appearances(&mut document)?;
        }
        "set_document_metadata" => {
            let title = optional_str(&operation.payload, "title")?;
            let author = optional_str(&operation.payload, "author")?;
            let subject = optional_str(&operation.payload, "subject")?;
            set_info_metadata(&mut document, title, author, subject)?;
        }
        "clear_document_metadata" => {
            clear_info_metadata(&mut document)?;
        }
        "set_form_field_readonly" => {
            let name = required_str(&operation.payload, "name")?;
            let readonly = operation
                .payload
                .get("readonly")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`readonly` boolean is required"))?;
            set_field_flag(&mut document, name, FIELD_FLAG_READONLY, readonly)?;
        }
        "set_form_field_required" => {
            let name = required_str(&operation.payload, "name")?;
            let required = operation
                .payload
                .get("required")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`required` boolean is required"))?;
            set_field_flag(&mut document, name, FIELD_FLAG_REQUIRED, required)?;
        }
        "set_form_field_multiline" => {
            let name = required_str(&operation.payload, "name")?;
            let multiline = operation
                .payload
                .get("multiline")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`multiline` boolean is required"))?;
            set_field_flag(&mut document, name, FIELD_FLAG_MULTILINE, multiline)?;
        }
        "set_form_field_password" => {
            let name = required_str(&operation.payload, "name")?;
            let password = operation
                .payload
                .get("password")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`password` boolean is required"))?;
            set_field_flag(&mut document, name, FIELD_FLAG_PASSWORD, password)?;
        }
        "set_form_field_max_length" => {
            let name = required_str(&operation.payload, "name")?;
            let max_length = match operation.payload.get("max_length") {
                None | Some(serde_json::Value::Null) => None,
                Some(value) => Some(
                    value
                        .as_u64()
                        .filter(|v| *v > 0 && *v <= u64::from(u32::MAX))
                        .and_then(|v| u32::try_from(v).ok())
                        .ok_or_else(|| {
                            format_error("`max_length` must be a positive integer or null")
                        })?,
                ),
            };
            set_field_max_length(&mut document, name, max_length)?;
        }
        "set_form_field_comb" => {
            let name = required_str(&operation.payload, "name")?;
            let comb = operation
                .payload
                .get("comb")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`comb` boolean is required"))?;
            set_field_flag(&mut document, name, FIELD_FLAG_COMB, comb)?;
        }
        "set_form_field_do_not_scroll" => {
            let name = required_str(&operation.payload, "name")?;
            let do_not_scroll = operation
                .payload
                .get("do_not_scroll")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`do_not_scroll` boolean is required"))?;
            set_field_flag(&mut document, name, FIELD_FLAG_DO_NOT_SCROLL, do_not_scroll)?;
        }
        "set_form_field_do_not_spell_check" => {
            let name = required_str(&operation.payload, "name")?;
            let do_not_spell_check = operation
                .payload
                .get("do_not_spell_check")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`do_not_spell_check` boolean is required"))?;
            set_field_flag(
                &mut document,
                name,
                FIELD_FLAG_DO_NOT_SPELL_CHECK,
                do_not_spell_check,
            )?;
        }
        "set_form_field_rich_text" => {
            let name = required_str(&operation.payload, "name")?;
            let rich_text = operation
                .payload
                .get("rich_text")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`rich_text` boolean is required"))?;
            set_field_flag(&mut document, name, FIELD_FLAG_RICH_TEXT, rich_text)?;
        }
        "set_form_field_no_export" => {
            let name = required_str(&operation.payload, "name")?;
            let no_export = operation
                .payload
                .get("no_export")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`no_export` boolean is required"))?;
            set_field_flag(&mut document, name, FIELD_FLAG_NO_EXPORT, no_export)?;
        }
        "set_form_field_multi_select" => {
            let name = required_str(&operation.payload, "name")?;
            let multi_select = operation
                .payload
                .get("multi_select")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`multi_select` boolean is required"))?;
            set_field_flag(&mut document, name, FIELD_FLAG_MULTI_SELECT, multi_select)?;
        }
        other => {
            return Err(format_error(format!(
                "unsupported pdf edit `{other}` in apply"
            )));
        }
    }
    let mut bytes = Vec::new();
    document
        .save_to(&mut Cursor::new(&mut bytes))
        .map_err(|error| format_error(format!("cannot save PDF: {error}")))?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn set_info_metadata(
    document: &mut Document,
    title: Option<&str>,
    author: Option<&str>,
    subject: Option<&str>,
) -> Result<()> {
    let info_id = ensure_info_dict(document)?;
    let object = document
        .get_object_mut(info_id)
        .map_err(|error| format_error(format!("cannot update Info: {error}")))?;
    let Object::Dictionary(dict) = object else {
        return Err(format_error("Info is not a dictionary"));
    };
    if let Some(title) = title {
        dict.set("Title", Object::string_literal(title));
    }
    if let Some(author) = author {
        dict.set("Author", Object::string_literal(author));
    }
    if let Some(subject) = subject {
        dict.set("Subject", Object::string_literal(subject));
    }
    Ok(())
}

fn clear_info_metadata(document: &mut Document) -> Result<()> {
    let Ok(info_id) = ensure_info_dict(document) else {
        return Ok(());
    };
    let object = document
        .get_object_mut(info_id)
        .map_err(|error| format_error(format!("cannot update Info: {error}")))?;
    let Object::Dictionary(dict) = object else {
        return Err(format_error("Info is not a dictionary"));
    };
    let _ = dict.remove(b"Title");
    let _ = dict.remove(b"Author");
    let _ = dict.remove(b"Subject");
    Ok(())
}

fn ensure_info_dict(document: &mut Document) -> Result<ObjectId> {
    match document.trailer.get(b"Info") {
        Ok(Object::Reference(id)) => Ok(*id),
        Ok(Object::Dictionary(_)) => {
            let dict = match document.trailer.remove(b"Info") {
                Some(Object::Dictionary(dict)) => dict,
                _ => Dictionary::new(),
            };
            let id = document.add_object(Object::Dictionary(dict));
            document.trailer.set("Info", Object::Reference(id));
            Ok(id)
        }
        Ok(_) => Err(format_error("Info entry has an unsupported type")),
        Err(_) => {
            let id = document.add_object(Object::Dictionary(Dictionary::new()));
            document.trailer.set("Info", Object::Reference(id));
            Ok(id)
        }
    }
}

fn resolve_btn_value(field: &PdfFieldModel, value: &str) -> Result<String> {
    let on_states: Vec<&str> = field
        .export_values
        .iter()
        .map(String::as_str)
        .filter(|state| *state != "Off")
        .collect();

    if value.eq_ignore_ascii_case("Off") {
        return Ok("Off".into());
    }
    if value.eq_ignore_ascii_case("On") {
        return match on_states.as_slice() {
            [only] => Ok((*only).to_owned()),
            [] => Ok("Yes".into()),
            _ => Err(format_error(
                "ambiguous radio: specify an explicit export value",
            )),
        };
    }
    if let Some(exact) = field
        .export_values
        .iter()
        .find(|state| state.as_str() == value)
    {
        return Ok(exact.clone());
    }
    if let Some(exact) = field
        .export_values
        .iter()
        .find(|state| state.eq_ignore_ascii_case(value))
    {
        return Ok(exact.clone());
    }
    if field.export_values.is_empty() && value.eq_ignore_ascii_case("Yes") {
        return Ok("Yes".into());
    }
    if on_states.len() > 1 {
        return Err(format_error(
            "ambiguous radio: specify an explicit export value",
        ));
    }
    Err(format_error(format!(
        "value `{value}` is not a valid checkbox/radio state"
    )))
}

fn set_field_value(
    document: &mut Document,
    name: &str,
    value: &str,
    field_type: &str,
) -> Result<()> {
    let mut target = None;
    for (id, object) in &document.objects {
        let Object::Dictionary(dict) = object else {
            continue;
        };
        let Ok(Object::String(bytes, _)) = dict.get(b"T") else {
            continue;
        };
        if String::from_utf8_lossy(bytes) == name {
            target = Some(*id);
            break;
        }
    }
    let id = target.ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    let kid_ids = kid_object_ids(document, id)?;
    let object = document
        .get_object_mut(id)
        .map_err(|error| format_error(format!("cannot update field: {error}")))?;
    let Object::Dictionary(dict) = object else {
        return Err(format_error("field is not a dictionary"));
    };
    if field_type == "btn" {
        let name_value = Object::Name(value.as_bytes().to_vec());
        dict.set("V", name_value.clone());
        if dict.has(b"AS") || kid_ids.is_empty() {
            dict.set("AS", name_value);
        }
    } else {
        dict.set("V", Object::string_literal(value));
    }
    if field_type == "btn" {
        update_widget_appearance_states(document, &kid_ids, value)?;
    }
    Ok(())
}

const FIELD_FLAG_READONLY: i64 = 1;
const FIELD_FLAG_REQUIRED: i64 = 2;
const FIELD_FLAG_NO_EXPORT: i64 = 8;
const FIELD_FLAG_MULTI_SELECT: i64 = 1_048_576;
const FIELD_FLAG_MULTILINE: i64 = 4096;
const FIELD_FLAG_PASSWORD: i64 = 8192;
const FIELD_FLAG_DO_NOT_SPELL_CHECK: i64 = 4_194_304;
const FIELD_FLAG_DO_NOT_SCROLL: i64 = 8_388_608;
const FIELD_FLAG_COMB: i64 = 16_777_216;
const FIELD_FLAG_RICH_TEXT: i64 = 33_554_432;

fn set_field_flag(document: &mut Document, name: &str, flag: i64, enabled: bool) -> Result<()> {
    let mut target = None;
    for (id, object) in &document.objects {
        let Object::Dictionary(dict) = object else {
            continue;
        };
        let Ok(Object::String(bytes, _)) = dict.get(b"T") else {
            continue;
        };
        if String::from_utf8_lossy(bytes) == name {
            target = Some(*id);
            break;
        }
    }
    let id = target.ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    let object = document
        .get_object_mut(id)
        .map_err(|error| format_error(format!("cannot update field: {error}")))?;
    let Object::Dictionary(dict) = object else {
        return Err(format_error("field is not a dictionary"));
    };
    let flags = dict
        .get(b"Ff")
        .ok()
        .and_then(|object| object.as_i64().ok())
        .unwrap_or(0);
    let updated = if enabled { flags | flag } else { flags & !flag };
    if updated == 0 {
        dict.remove(b"Ff");
    } else {
        dict.set("Ff", Object::Integer(updated));
    }
    Ok(())
}

fn set_field_max_length(
    document: &mut Document,
    name: &str,
    max_length: Option<u32>,
) -> Result<()> {
    let mut target = None;
    for (id, object) in &document.objects {
        let Object::Dictionary(dict) = object else {
            continue;
        };
        let Ok(Object::String(bytes, _)) = dict.get(b"T") else {
            continue;
        };
        if String::from_utf8_lossy(bytes) == name {
            target = Some(*id);
            break;
        }
    }
    let id = target.ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    let object = document
        .get_object_mut(id)
        .map_err(|error| format_error(format!("cannot update field: {error}")))?;
    let Object::Dictionary(dict) = object else {
        return Err(format_error("field is not a dictionary"));
    };
    match max_length {
        Some(value) => dict.set("MaxLen", Object::Integer(i64::from(value))),
        None => {
            dict.remove(b"MaxLen");
        }
    }
    Ok(())
}

fn kid_object_ids(document: &Document, field_id: ObjectId) -> Result<Vec<ObjectId>> {
    let object = document
        .get_object(field_id)
        .map_err(|error| format_error(format!("cannot read field: {error}")))?;
    let Object::Dictionary(dict) = object else {
        return Ok(Vec::new());
    };
    let Ok(kids) = dict.get(b"Kids") else {
        return Ok(Vec::new());
    };
    let Object::Array(items) = kids else {
        return Ok(Vec::new());
    };
    Ok(items
        .iter()
        .filter_map(|item| match item {
            Object::Reference(id) => Some(*id),
            _ => None,
        })
        .collect())
}

fn update_widget_appearance_states(
    document: &mut Document,
    kid_ids: &[ObjectId],
    value: &str,
) -> Result<()> {
    for kid_id in kid_ids {
        let states = widget_on_states(document, *kid_id)?;
        let appearance = if states.iter().any(|state| state == value) {
            value
        } else {
            "Off"
        };
        let object = document
            .get_object_mut(*kid_id)
            .map_err(|error| format_error(format!("cannot update widget: {error}")))?;
        let Object::Dictionary(dict) = object else {
            continue;
        };
        dict.set("AS", Object::Name(appearance.as_bytes().to_vec()));
    }
    Ok(())
}

fn widget_on_states(document: &Document, kid_id: ObjectId) -> Result<Vec<String>> {
    let object = document
        .get_object(kid_id)
        .map_err(|error| format_error(format!("cannot read widget: {error}")))?;
    let Object::Dictionary(dict) = object else {
        return Ok(Vec::new());
    };
    Ok(ap_n_keys(document, dict))
}

fn ap_n_keys(document: &Document, dict: &Dictionary) -> Vec<String> {
    let Ok(ap) = dict.get(b"AP") else {
        return Vec::new();
    };
    let Some(ap_dict) = resolve_dict(document, ap) else {
        return Vec::new();
    };
    let Ok(normal) = ap_dict.get(b"N") else {
        return Vec::new();
    };
    let Some(normal_dict) = resolve_dict(document, normal) else {
        return Vec::new();
    };
    normal_dict
        .iter()
        .map(|(key, _)| String::from_utf8_lossy(key).into_owned())
        .filter(|name| name != "Off")
        .collect()
}

fn resolve_dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Reference(id) => match document.get_object(*id).ok()? {
            Object::Dictionary(dict) => Some(dict),
            _ => None,
        },
        _ => None,
    }
}

fn set_need_appearances(document: &mut Document) -> Result<()> {
    let catalog = document
        .catalog()
        .map_err(|error| format_error(format!("missing catalog: {error}")))?;
    let Ok(Object::Reference(form_id)) = catalog.get(b"AcroForm") else {
        return Ok(());
    };
    let form_id = *form_id;
    let object = document
        .get_object_mut(form_id)
        .map_err(|error| format_error(format!("cannot update AcroForm: {error}")))?;
    if let Object::Dictionary(dict) = object {
        dict.set("NeedAppearances", true);
    }
    Ok(())
}

fn required_str<'a>(payload: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format_error(format!("`{key}` is required")))
}

fn required_bool(payload: &serde_json::Value, key: &str) -> Result<bool> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| format_error(format!("`{key}` boolean is required")))
}

fn optional_str<'a>(payload: &'a serde_json::Value, key: &str) -> Result<Option<&'a str>> {
    match payload.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(format_error(format!("`{key}` must be a string"))),
    }
}

fn optional_positive_u32(payload: &serde_json::Value, key: &str) -> Result<Option<u32>> {
    match payload.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|v| *v > 0 && *v <= u64::from(u32::MAX))
            .and_then(|v| u32::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| format_error(format!("`{key}` must be a positive integer or null"))),
    }
}

fn format_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<pdf edit>".into(),
        message: message.into(),
    }
}
