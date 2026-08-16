use std::io::Cursor;

use dotall_core::{
    DependencyImpact, DotallError, PatchedOutput, Result, SemanticChange, SemanticOperation,
    ValidatedEdit,
};
use lopdf::{Document, Object};

use crate::FORMAT_ID;
use crate::model::{PdfDocumentModel, SCHEMA_ID, SCHEMA_VERSION};

pub fn validate(
    model: &PdfDocumentModel,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 || operations[0].kind != "set_form_field" {
        return Err(format_error(
            "pdf v0 supports a single set_form_field operation per transaction",
        ));
    }
    if model.encrypted {
        return Err(format_error("cannot edit encrypted PDF"));
    }
    if model.fields.iter().any(|field| field.field_type == "sig") {
        return Err(format_error("cannot edit signed PDF"));
    }
    let operation = &operations[0];
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
    if field.field_type == "btn" {
        return Err(format_error("checkbox/radio not supported in v0"));
    }
    if field.field_type != "tx" && field.field_type != "ch" {
        return Err(format_error(format!(
            "field type `{}` is not supported in v0",
            field.field_type
        )));
    }
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
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: field.name.clone(),
            element_id: field.element_id.clone(),
            change: "set_form_field".into(),
            before: Some(field.value.clone()),
            after: Some(value.to_owned()),
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
    let name = required_str(&operation.payload, "name")?;
    let value = required_str(&operation.payload, "value")?;
    let mut document = Document::load_mem(package)
        .map_err(|error| format_error(format!("invalid PDF: {error}")))?;
    set_field_value(&mut document, name, value)?;
    set_need_appearances(&mut document)?;
    let mut bytes = Vec::new();
    document
        .save_to(&mut Cursor::new(&mut bytes))
        .map_err(|error| format_error(format!("cannot save PDF: {error}")))?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn set_field_value(document: &mut Document, name: &str, value: &str) -> Result<()> {
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
    dict.set("V", Object::string_literal(value));
    Ok(())
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

fn format_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<pdf edit>".into(),
        message: message.into(),
    }
}
