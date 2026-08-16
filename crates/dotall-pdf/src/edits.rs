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
    let field_type = operation
        .payload
        .get("field_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("tx");
    let mut document = Document::load_mem(package)
        .map_err(|error| format_error(format!("invalid PDF: {error}")))?;
    set_field_value(&mut document, name, value, field_type)?;
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

fn format_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<pdf edit>".into(),
        message: message.into(),
    }
}
