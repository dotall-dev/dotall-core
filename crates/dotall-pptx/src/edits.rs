use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use dotall_core::{
    DependencyImpact, DotallError, PatchedOutput, Result, SemanticChange, SemanticOperation,
    ValidatedEdit,
};
use zip::ZipArchive;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::FORMAT_ID;
use crate::model::{PresentationModel, SCHEMA_ID, SCHEMA_VERSION};
use crate::selector;

pub fn validate(
    model: &PresentationModel,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 || operations[0].kind != "set_shape_text" {
        return Err(format_error(
            "pptx v0 supports a single set_shape_text operation per transaction",
        ));
    }
    let operation = &operations[0];
    let slide_ref = required_str(&operation.payload, "slide")?;
    let shape_ref = required_str(&operation.payload, "shape")?;
    let text = required_str(&operation.payload, "text")?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    let shape = slide
        .shapes
        .iter()
        .find(|shape| shape.name == shape_ref || shape.element_id == shape_ref)
        .ok_or_else(|| format_error(format!("shape `{shape_ref}` was not found")))?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_shape_text".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "shape": shape.name,
                "text": text,
                "part_name": slide.part_name,
                "element_id": shape.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}!{}", slide.name, shape.name),
            element_id: shape.element_id.clone(),
            change: "set_shape_text".into(),
            before: Some(shape.text.clone()),
            after: Some(text.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

pub fn apply_bytes(package: &[u8], edit: &ValidatedEdit) -> Result<PatchedOutput> {
    let operation = edit
        .operations
        .first()
        .ok_or_else(|| format_error("validated edit is missing operations"))?;
    let part_name = required_str(&operation.payload, "part_name")?;
    let shape = required_str(&operation.payload, "shape")?;
    let text = required_str(&operation.payload, "text")?;
    let original = entry_bytes(package, part_name)?;
    let patched_xml = patch_shape_text(&original, shape, text)?;
    let bytes = rebuild_package(
        package,
        &BTreeMap::from([(part_name.to_owned(), patched_xml)]),
    )?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

pub fn patch_shape_text(xml: &[u8], shape: &str, text: &str) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("slide XML is not UTF-8: {error}")))?;
    let needle = format!("name=\"{shape}\"");
    let name_at = source
        .find(&needle)
        .ok_or_else(|| format_error(format!("shape `{shape}` was not found in slide XML")))?;
    let sp_start = source[..name_at]
        .rfind("<p:sp")
        .ok_or_else(|| format_error("shape is missing a p:sp wrapper"))?;
    let rel_end = source[name_at..]
        .find("</p:sp>")
        .ok_or_else(|| format_error("shape is missing a closing p:sp"))?;
    let sp_end = name_at + rel_end + "</p:sp>".len();
    let sp = &source[sp_start..sp_end];
    if sp.contains("<p:graphicFrame")
        || sp.contains("<a:graphic")
        || sp.contains("<mc:AlternateContent")
    {
        return Err(format_error("cannot edit non-text shape"));
    }
    let Some(t_rel) = sp.find("<a:t") else {
        return Err(format_error("shape has no text run to patch"));
    };
    let t_open_end = sp[t_rel..]
        .find('>')
        .map(|offset| t_rel + offset + 1)
        .ok_or_else(|| format_error("unterminated a:t"))?;
    let t_close_rel = sp[t_open_end..]
        .find("</a:t>")
        .ok_or_else(|| format_error("unterminated a:t content"))?;
    let t_close = t_open_end + t_close_rel;
    let mut patched_sp = String::new();
    patched_sp.push_str(&sp[..t_open_end]);
    patched_sp.push_str(&xml_escape(text));
    patched_sp.push_str(&sp[t_close..]);
    // Drop extra a:t contents after the first run.
    patched_sp = clear_later_text_runs(&patched_sp);
    let mut output = String::new();
    output.push_str(&source[..sp_start]);
    output.push_str(&patched_sp);
    output.push_str(&source[sp_end..]);
    Ok(output.into_bytes())
}

fn clear_later_text_runs(sp: &str) -> String {
    let Some(first_close) = sp.find("</a:t>") else {
        return sp.to_owned();
    };
    let mut cursor = first_close + "</a:t>".len();
    let mut output = sp[..cursor].to_owned();
    while let Some(rel) = sp[cursor..].find("<a:t") {
        let start = cursor + rel;
        let Some(open_end_rel) = sp[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel + 1;
        let Some(close_rel) = sp[open_end..].find("</a:t>") else {
            break;
        };
        output.push_str(&sp[cursor..open_end]);
        output.push_str(&sp[open_end + close_rel..open_end + close_rel + "</a:t>".len()]);
        cursor = open_end + close_rel + "</a:t>".len();
    }
    output.push_str(&sp[cursor..]);
    output
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn entry_bytes(package: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(format!("invalid PPTX package: {error}")))?;
    let mut entry = archive
        .by_name(name)
        .map_err(|error| format_error(format!("missing `{name}`: {error}")))?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes)
        .map_err(|error| format_error(format!("cannot read `{name}`: {error}")))?;
    Ok(bytes)
}

fn rebuild_package(original: &[u8], replacements: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(original))
        .map_err(|error| format_error(format!("invalid PPTX package: {error}")))?;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format_error(format!("cannot read ZIP entry: {error}")))?;
        let name = entry.name().to_owned();
        if let Some(replacement) = replacements.get(&name) {
            let options = SimpleFileOptions::default()
                .compression_method(entry.compression())
                .last_modified_time(entry.last_modified().unwrap_or_default());
            writer.start_file(&name, options).map_err(|error| {
                format_error(format!("cannot start patched ZIP entry: {error}"))
            })?;
            writer.write_all(replacement).map_err(|error| {
                format_error(format!("cannot write patched ZIP entry: {error}"))
            })?;
        } else {
            writer
                .raw_copy_file(entry)
                .map_err(|error| format_error(format!("cannot copy ZIP entry: {error}")))?;
        }
    }
    writer
        .finish()
        .map_err(|error| format_error(format!("cannot finish PPTX package: {error}")))
        .map(|cursor| cursor.into_inner())
}

fn required_str<'a>(payload: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format_error(format!("`{key}` is required")))
}

pub fn apply(source: &std::path::Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
    let bytes = std::fs::read(source).map_err(|error| DotallError::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    apply_bytes(&bytes, edit)
}

fn format_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<pptx edit>".into(),
        message: message.into(),
    }
}
