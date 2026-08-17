use std::io::{Cursor, Read};

use base64::Engine;
use dotall_core::{
    DependencyImpact, DotallError, PatchedOutput, Result, SemanticChange, SemanticOperation,
    ValidatedEdit,
};
use flate2::read::ZlibDecoder;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

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
        "set_form_field_combo" => validate_set_form_field_combo(model, operation),
        "set_form_field_edit" => validate_set_form_field_edit(model, operation),
        "set_document_metadata" => validate_set_document_metadata(model, operation),
        "clear_document_metadata" => validate_clear_document_metadata(model, operation),
        "insert_comment" => validate_insert_comment(model, operation),
        "insert_picture" => validate_insert_picture(model, operation),
        "set_comment" | "delete_comment" | "replace_comment" => Err(format_error(format!(
            "unsupported pdf edit `{}`; mutate-existing comment ops are rejected — use insert_comment",
            operation.kind
        ))),
        "draw_image" | "replace_picture" | "delete_picture" => Err(format_error(format!(
            "unsupported pdf edit `{}`; mutate/draw picture ops are rejected — use insert_picture (stamp annotation)",
            operation.kind
        ))),
        other => Err(format_error(format!(
            "unsupported pdf edit `{other}`; use set_form_field, clear_form_field, clear_all_form_fields, set_form_fields, set_form_field_readonly, set_form_field_required, set_form_field_multiline, set_form_field_password, set_form_field_max_length, set_form_field_comb, set_form_field_do_not_scroll, set_form_field_do_not_spell_check, set_form_field_rich_text, set_form_field_no_export, set_form_field_multi_select, set_form_field_combo, set_form_field_edit, set_document_metadata, clear_document_metadata, insert_comment, or insert_picture"
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

fn validate_set_form_field_combo(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let combo = required_bool(&operation.payload, "combo")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type != "ch" {
        return Err(format_error(
            "set_form_field_combo only applies to choice (ch) fields",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_combo".into(),
            payload: serde_json::json!({
                "name": field.name,
                "combo": combo,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_combo".into(),
            before: Some(if field.combo { "true" } else { "false" }.into()),
            after: Some(if combo { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_form_field_edit(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let name = required_str(&operation.payload, "name")?;
    let edit = required_bool(&operation.payload, "edit")?;
    let field = model
        .fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| format_error(format!("field `{name}` was not found")))?;
    if field.field_type != "ch" {
        return Err(format_error(
            "set_form_field_edit only applies to choice (ch) fields",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_form_field_edit".into(),
            payload: serde_json::json!({
                "name": field.name,
                "edit": edit,
                "element_id": field.element_id,
                "field_type": field.field_type,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("field:{}", field.name),
            element_id: field.element_id.clone(),
            change: "set_form_field_edit".into(),
            before: Some(if field.edit { "true" } else { "false" }.into()),
            after: Some(if edit { "true" } else { "false" }.into()),
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

fn validate_insert_comment(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let page = required_page(&operation.payload)?;
    if page == 0 || page > model.page_count {
        return Err(format_error(format!(
            "page `{page}` is out of range (1..={})",
            model.page_count
        )));
    }
    let contents = required_str(&operation.payload, "contents")?;
    if contents.is_empty() {
        return Err(format_error("`contents` must not be empty"));
    }
    let author = optional_str(&operation.payload, "author")?.unwrap_or("Dotall");
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "insert_comment".into(),
            payload: serde_json::json!({
                "page": page,
                "contents": contents,
                "author": author,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("page:{page}"),
            element_id: model
                .pages
                .iter()
                .find(|p| p.number == page)
                .map(|p| p.element_id.clone())
                .unwrap_or_else(|| model.document_id.clone()),
            change: "insert_comment".into(),
            before: None,
            after: Some(contents.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_insert_picture(
    model: &PdfDocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let page = required_page(&operation.payload)?;
    if page == 0 || page > model.page_count {
        return Err(format_error(format!(
            "page `{page}` is out of range (1..={})",
            model.page_count
        )));
    }
    let encoded = required_str(&operation.payload, "bytes_base64")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format_error(format!("invalid bytes_base64: {error}")))?;
    if bytes.is_empty() {
        return Err(format_error("`bytes_base64` must not be empty"));
    }
    let content_type = optional_str(&operation.payload, "content_type")?.unwrap_or("image/png");
    let content_type = normalize_image_content_type(content_type)?;
    // Validate decode early so agents get a clear error before apply.
    let _image = decode_stamp_image(&bytes, content_type)?;
    let rect = optional_rect(&operation.payload)?.unwrap_or([400.0, 700.0, 500.0, 780.0]);
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "insert_picture".into(),
            payload: serde_json::json!({
                "page": page,
                "bytes_base64": encoded,
                "content_type": content_type,
                "rect": rect,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("page:{page}"),
            element_id: model
                .pages
                .iter()
                .find(|p| p.number == page)
                .map(|p| p.element_id.clone())
                .unwrap_or_else(|| model.document_id.clone()),
            change: "insert_picture".into(),
            before: None,
            after: Some(format!("stamp:{content_type}")),
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
        "set_form_field_combo" => {
            let name = required_str(&operation.payload, "name")?;
            let combo = operation
                .payload
                .get("combo")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`combo` boolean is required"))?;
            set_field_flag(&mut document, name, FIELD_FLAG_COMBO, combo)?;
        }
        "set_form_field_edit" => {
            let name = required_str(&operation.payload, "name")?;
            let edit = operation
                .payload
                .get("edit")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`edit` boolean is required"))?;
            set_field_flag(&mut document, name, FIELD_FLAG_EDIT, edit)?;
        }
        "insert_comment" => {
            let page = required_page(&operation.payload)?;
            let contents = required_str(&operation.payload, "contents")?;
            let author = optional_str(&operation.payload, "author")?.unwrap_or("Dotall");
            insert_text_annot(&mut document, page, contents, author)?;
        }
        "insert_picture" => {
            let page = required_page(&operation.payload)?;
            let encoded = required_str(&operation.payload, "bytes_base64")?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| format_error(format!("invalid bytes_base64: {error}")))?;
            let content_type =
                optional_str(&operation.payload, "content_type")?.unwrap_or("image/png");
            let content_type = normalize_image_content_type(content_type)?;
            let rect = optional_rect(&operation.payload)?.unwrap_or([400.0, 700.0, 500.0, 780.0]);
            insert_stamp_annot(&mut document, page, &bytes, content_type, rect)?;
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
const FIELD_FLAG_COMBO: i64 = 131_072;
const FIELD_FLAG_EDIT: i64 = 262_144;
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

fn insert_text_annot(
    document: &mut Document,
    page_number: u32,
    contents: &str,
    author: &str,
) -> Result<()> {
    let pages = document.get_pages();
    let page_id = pages
        .get(&page_number)
        .copied()
        .ok_or_else(|| format_error(format!("page `{page_number}` was not found in the PDF")))?;
    let rect = sticky_rect_for_page(document, page_id);
    let mut annot = Dictionary::new();
    annot.set("Type", Object::Name(b"Annot".to_vec()));
    annot.set("Subtype", Object::Name(b"Text".to_vec()));
    annot.set("Contents", Object::string_literal(contents));
    annot.set("T", Object::string_literal(author));
    annot.set("Rect", Object::Array(rect));
    annot.set("P", Object::Reference(page_id));
    annot.set(
        "C",
        Object::Array(vec![
            Object::Real(1.0),
            Object::Real(1.0),
            Object::Real(0.0),
        ]),
    );
    annot.set("Open", Object::Boolean(false));
    annot.set("Name", Object::Name(b"Comment".to_vec()));
    let annot_id = document.add_object(Object::Dictionary(annot));
    append_page_annot(document, page_id, annot_id)
}

fn insert_stamp_annot(
    document: &mut Document,
    page_number: u32,
    image_bytes: &[u8],
    content_type: &str,
    rect: [f32; 4],
) -> Result<()> {
    let pages = document.get_pages();
    let page_id = pages
        .get(&page_number)
        .copied()
        .ok_or_else(|| format_error(format!("page `{page_number}` was not found in the PDF")))?;
    let image = decode_stamp_image(image_bytes, content_type)?;
    let width = (rect[2] - rect[0]).abs().max(1.0);
    let height = (rect[3] - rect[1]).abs().max(1.0);

    let mut image_dict = Dictionary::new();
    image_dict.set("Type", Object::Name(b"XObject".to_vec()));
    image_dict.set("Subtype", Object::Name(b"Image".to_vec()));
    image_dict.set("Width", Object::Integer(i64::from(image.width)));
    image_dict.set("Height", Object::Integer(i64::from(image.height)));
    image_dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
    image_dict.set("BitsPerComponent", Object::Integer(8));
    if image.dct_decode {
        image_dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
    }
    let image_id = document.add_object(Object::Stream(Stream::new(image_dict, image.pixels)));

    let mut resources = Dictionary::new();
    let mut xobjects = Dictionary::new();
    xobjects.set("Im0", Object::Reference(image_id));
    resources.set("XObject", Object::Dictionary(xobjects));

    let form_content = format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q\n");
    let mut form_dict = Dictionary::new();
    form_dict.set("Type", Object::Name(b"XObject".to_vec()));
    form_dict.set("Subtype", Object::Name(b"Form".to_vec()));
    form_dict.set(
        "BBox",
        Object::Array(vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(width),
            Object::Real(height),
        ]),
    );
    form_dict.set("Resources", Object::Dictionary(resources));
    let form_id = document.add_object(Object::Stream(Stream::new(
        form_dict,
        form_content.into_bytes(),
    )));

    let mut ap = Dictionary::new();
    ap.set("N", Object::Reference(form_id));

    let mut annot = Dictionary::new();
    annot.set("Type", Object::Name(b"Annot".to_vec()));
    annot.set("Subtype", Object::Name(b"Stamp".to_vec()));
    annot.set(
        "Rect",
        Object::Array(vec![
            Object::Real(rect[0]),
            Object::Real(rect[1]),
            Object::Real(rect[2]),
            Object::Real(rect[3]),
        ]),
    );
    annot.set("P", Object::Reference(page_id));
    annot.set("AP", Object::Dictionary(ap));
    annot.set("F", Object::Integer(4));
    let annot_id = document.add_object(Object::Dictionary(annot));
    append_page_annot(document, page_id, annot_id)
}

fn append_page_annot(document: &mut Document, page_id: ObjectId, annot_id: ObjectId) -> Result<()> {
    // Resolve /Annots when stored as an indirect array so we only append a ref.
    let annots_target = {
        let page_obj = document
            .get_object(page_id)
            .map_err(|error| format_error(format!("cannot read page: {error}")))?;
        let Object::Dictionary(page_dict) = page_obj else {
            return Err(format_error("page is not a dictionary"));
        };
        match page_dict.get(b"Annots") {
            Ok(Object::Reference(id)) => Some(AnnotsTarget::Indirect(*id)),
            Ok(Object::Array(_)) => Some(AnnotsTarget::Inline),
            Ok(_) => {
                return Err(format_error("page /Annots has an unsupported type"));
            }
            Err(_) => None,
        }
    };

    match annots_target {
        Some(AnnotsTarget::Indirect(array_id)) => {
            let object = document
                .get_object_mut(array_id)
                .map_err(|error| format_error(format!("cannot update Annots: {error}")))?;
            let Object::Array(items) = object else {
                return Err(format_error("page /Annots reference is not an array"));
            };
            items.push(Object::Reference(annot_id));
        }
        Some(AnnotsTarget::Inline) | None => {
            let page_obj = document
                .get_object_mut(page_id)
                .map_err(|error| format_error(format!("cannot update page: {error}")))?;
            let Object::Dictionary(page_dict) = page_obj else {
                return Err(format_error("page is not a dictionary"));
            };
            match page_dict.get_mut(b"Annots") {
                Ok(Object::Array(items)) => {
                    items.push(Object::Reference(annot_id));
                }
                Ok(_) => {
                    return Err(format_error("page /Annots is not an array"));
                }
                Err(_) => {
                    page_dict.set("Annots", Object::Array(vec![Object::Reference(annot_id)]));
                }
            }
        }
    }
    Ok(())
}

enum AnnotsTarget {
    Inline,
    Indirect(ObjectId),
}

struct DecodedStampImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    dct_decode: bool,
}

fn normalize_image_content_type(content_type: &str) -> Result<&'static str> {
    match content_type {
        "image/png" | "png" => Ok("image/png"),
        "image/jpeg" | "image/jpg" | "jpeg" | "jpg" => Ok("image/jpeg"),
        other => Err(format_error(format!(
            "insert_picture supports PNG IHDR 8-bit RGB / JPEG; unsupported content_type `{other}`"
        ))),
    }
}

fn decode_stamp_image(bytes: &[u8], content_type: &str) -> Result<DecodedStampImage> {
    match content_type {
        "image/png" => decode_png_ihdr_8bit_rgb_1x1(bytes),
        "image/jpeg" => decode_jpeg_dct(bytes),
        other => Err(format_error(format!(
            "insert_picture supports PNG IHDR 8-bit RGB / JPEG; unsupported content_type `{other}`"
        ))),
    }
}

/// Tiny PNG decoder: only 1×1 8-bit RGB (color type 2), non-interlaced.
fn decode_png_ihdr_8bit_rgb_1x1(bytes: &[u8]) -> Result<DecodedStampImage> {
    const SIG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 8 || &bytes[..8] != SIG {
        return Err(format_error(
            "insert_picture supports PNG IHDR 8-bit RGB / JPEG; invalid PNG signature",
        ));
    }
    let mut offset = 8;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut interlace = 0u8;
    let mut saw_ihdr = false;
    let mut idat = Vec::new();
    while offset + 8 <= bytes.len() {
        let len = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let chunk_type = &bytes[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start
            .checked_add(len)
            .ok_or_else(|| format_error("invalid PNG chunk length"))?;
        if data_end + 4 > bytes.len() {
            return Err(format_error("truncated PNG chunk"));
        }
        let data = &bytes[data_start..data_end];
        match chunk_type {
            b"IHDR" => {
                if len < 13 {
                    return Err(format_error(
                        "insert_picture supports PNG IHDR 8-bit RGB / JPEG; invalid IHDR",
                    ));
                }
                width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                bit_depth = data[8];
                color_type = data[9];
                interlace = data[12];
                saw_ihdr = true;
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        offset = data_end + 4;
    }
    if !saw_ihdr {
        return Err(format_error(
            "insert_picture supports PNG IHDR 8-bit RGB / JPEG; missing IHDR",
        ));
    }
    if width != 1 || height != 1 || bit_depth != 8 || color_type != 2 || interlace != 0 {
        return Err(format_error(
            "insert_picture supports PNG IHDR 8-bit RGB (1×1 non-interlaced) / JPEG",
        ));
    }
    if idat.is_empty() {
        return Err(format_error(
            "insert_picture supports PNG IHDR 8-bit RGB / JPEG; missing IDAT",
        ));
    }
    let mut decoder = ZlibDecoder::new(idat.as_slice());
    let mut inflated = Vec::new();
    decoder
        .read_to_end(&mut inflated)
        .map_err(|error| format_error(format!("invalid PNG IDAT: {error}")))?;
    // Filter byte + RGB
    if inflated.len() < 4 {
        return Err(format_error(
            "insert_picture supports PNG IHDR 8-bit RGB / JPEG; truncated scanline",
        ));
    }
    let pixels = inflated[1..4].to_vec();
    Ok(DecodedStampImage {
        width: 1,
        height: 1,
        pixels,
        dct_decode: false,
    })
}

fn decode_jpeg_dct(bytes: &[u8]) -> Result<DecodedStampImage> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return Err(format_error(
            "insert_picture supports PNG IHDR 8-bit RGB / JPEG; invalid JPEG SOI",
        ));
    }
    let (width, height) = jpeg_dimensions(bytes)?;
    Ok(DecodedStampImage {
        width,
        height,
        pixels: bytes.to_vec(),
        dct_decode: true,
    })
}

fn jpeg_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    let mut i = 2usize;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        if i + 4 > bytes.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if seg_len < 2 || i + 2 + seg_len > bytes.len() {
            return Err(format_error(
                "insert_picture supports PNG IHDR 8-bit RGB / JPEG; truncated JPEG segment",
            ));
        }
        // SOF0 / SOF1 / SOF2
        if matches!(marker, 0xC0..=0xC2) && seg_len >= 7 {
            let height = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let width = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            if width == 0 || height == 0 {
                return Err(format_error(
                    "insert_picture supports PNG IHDR 8-bit RGB / JPEG; invalid JPEG dimensions",
                ));
            }
            return Ok((width, height));
        }
        i += 2 + seg_len;
    }
    Err(format_error(
        "insert_picture supports PNG IHDR 8-bit RGB / JPEG; missing JPEG SOF",
    ))
}

fn sticky_rect_for_page(document: &Document, page_id: ObjectId) -> Vec<Object> {
    let media = document
        .get_object(page_id)
        .ok()
        .and_then(|object| match object {
            Object::Dictionary(dict) => dict.get(b"MediaBox").ok().cloned(),
            _ => None,
        });
    let (x1, y1, x2, y2) = match media {
        Some(Object::Array(values)) if values.len() == 4 => {
            let nums: Vec<f32> = values
                .iter()
                .map(|v| match v {
                    Object::Integer(n) => *n as f32,
                    Object::Real(n) => *n,
                    _ => 0.0,
                })
                .collect();
            (nums[0], nums[1], nums[2], nums[3])
        }
        _ => (0.0, 0.0, 612.0, 792.0),
    };
    let width = 24.0;
    let height = 24.0;
    let left = x1 + 12.0;
    let top = y2 - 12.0;
    let bottom = (top - height).max(y1);
    let right = (left + width).min(x2);
    vec![
        Object::Real(left),
        Object::Real(bottom),
        Object::Real(right),
        Object::Real(top),
    ]
}

fn required_page(payload: &serde_json::Value) -> Result<u32> {
    match payload.get("page") {
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| format_error("`page` must be a positive integer")),
        Some(_) => Err(format_error("`page` must be a positive integer")),
        None => Err(format_error("`page` is required")),
    }
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

fn optional_rect(payload: &serde_json::Value) -> Result<Option<[f32; 4]>> {
    match payload.get("rect") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Array(items)) if items.len() == 4 => {
            let mut rect = [0.0f32; 4];
            for (index, item) in items.iter().enumerate() {
                let Some(number) = item.as_f64() else {
                    return Err(format_error("`rect` must be an array of 4 numbers"));
                };
                rect[index] = number as f32;
            }
            Ok(Some(rect))
        }
        Some(_) => Err(format_error("`rect` must be an array of 4 numbers")),
    }
}

fn format_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<pdf edit>".into(),
        message: message.into(),
    }
}
