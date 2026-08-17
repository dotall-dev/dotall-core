use std::path::Path;

use dotall_core::{DotallError, Result};
use lopdf::{Document, Object};

use crate::FORMAT_ID;
use crate::ids;
use crate::model::{
    PdfCommentModel, PdfDocumentModel, PdfFieldModel, PdfMetadata, PdfPageModel, SCHEMA_VERSION,
};

pub fn parse_pdf(source: &Path) -> Result<PdfDocumentModel> {
    let bytes = std::fs::read(source).map_err(|error| DotallError::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    parse_pdf_bytes(&bytes)
}

pub fn parse_pdf_bytes(bytes: &[u8]) -> Result<PdfDocumentModel> {
    let source_hash = blake3::hash(bytes).to_hex().to_string();
    let document = match Document::load_mem(bytes) {
        Ok(document) => document,
        Err(error) => {
            if bytes.windows(8).any(|window| window == b"/Encrypt") {
                return Ok(encrypted_stub(&source_hash));
            }
            return Err(format_error(format!("invalid PDF: {error}")));
        }
    };
    let encrypted = document.trailer.get(b"Encrypt").ok().is_some();
    if encrypted {
        let mut model = encrypted_stub(&source_hash);
        model.page_count = document.get_pages().len() as u32;
        return Ok(model);
    }

    let pages_map = document.get_pages();
    let mut pages = Vec::new();
    for (number, page_id) in &pages_map {
        let text = page_text(&document, *page_id, *number);
        pages.push(PdfPageModel {
            element_id: ids::page_id(*number, SCHEMA_VERSION),
            number: *number,
            text,
        });
    }
    pages.sort_by_key(|page| page.number);

    let mut fields = Vec::new();
    collect_fields(&document, &mut fields)?;
    let comments = collect_comments(&document, &pages_map)?;
    let outline = collect_outline(&document);
    let metadata = collect_metadata(&document);

    Ok(PdfDocumentModel {
        document_id: ids::document_id(&source_hash, SCHEMA_VERSION),
        page_count: pages.len() as u32,
        pages,
        fields,
        comments,
        outline,
        encrypted: false,
        metadata,
    })
}

fn page_text(document: &Document, page_id: lopdf::ObjectId, number: u32) -> String {
    crate::text::page_text(document, page_id, number)
}

fn encrypted_stub(source_hash: &str) -> PdfDocumentModel {
    PdfDocumentModel {
        document_id: ids::document_id(source_hash, SCHEMA_VERSION),
        page_count: 0,
        pages: Vec::new(),
        fields: Vec::new(),
        comments: Vec::new(),
        outline: Vec::new(),
        encrypted: true,
        metadata: PdfMetadata::default(),
    }
}

fn collect_metadata(document: &Document) -> PdfMetadata {
    let Ok(info_obj) = document.trailer.get(b"Info") else {
        return PdfMetadata::default();
    };
    let Some(dict) = resolve_dict(document, info_obj).ok().flatten() else {
        return PdfMetadata::default();
    };
    PdfMetadata {
        title: dict.get(b"Title").ok().and_then(object_string),
        author: dict.get(b"Author").ok().and_then(object_string),
        subject: dict.get(b"Subject").ok().and_then(object_string),
        creator: dict.get(b"Creator").ok().and_then(object_string),
        producer: dict.get(b"Producer").ok().and_then(object_string),
    }
}

fn collect_fields(document: &Document, fields: &mut Vec<PdfFieldModel>) -> Result<()> {
    let Ok(catalog) = document.catalog() else {
        return Ok(());
    };
    let Ok(form_ref) = catalog.get(b"AcroForm") else {
        return Ok(());
    };
    let form = resolve_dict(document, form_ref)?;
    let Some(form) = form else {
        return Ok(());
    };
    let Ok(list) = form.get(b"Fields") else {
        return Ok(());
    };
    walk_field_list(document, list, "", fields)
}

fn collect_comments(
    document: &Document,
    pages_map: &std::collections::BTreeMap<u32, lopdf::ObjectId>,
) -> Result<Vec<PdfCommentModel>> {
    let mut comments = Vec::new();
    let mut page_numbers: Vec<_> = pages_map.keys().copied().collect();
    page_numbers.sort_unstable();
    for number in page_numbers {
        let Some(page_id) = pages_map.get(&number) else {
            continue;
        };
        let Ok(page_obj) = document.get_object(*page_id) else {
            continue;
        };
        let Object::Dictionary(page_dict) = page_obj else {
            continue;
        };
        let Ok(annots) = page_dict.get(b"Annots") else {
            continue;
        };
        let Ok(Object::Array(items)) = dereference(document, annots) else {
            continue;
        };
        for item in items {
            let object_id = match item {
                Object::Reference(id) => *id,
                _ => continue,
            };
            let Some(dict) = resolve_dict(document, item)? else {
                continue;
            };
            let Some(subtype) = dict.get(b"Subtype").ok().and_then(object_name) else {
                continue;
            };
            if subtype == "Widget" {
                continue;
            }
            if subtype != "Text" && subtype != "FreeText" {
                continue;
            }
            let contents = dict
                .get(b"Contents")
                .ok()
                .and_then(object_string)
                .unwrap_or_default();
            let author = dict
                .get(b"T")
                .ok()
                .and_then(object_string)
                .unwrap_or_default();
            let object_key = format!("{}:{}", object_id.0, object_id.1);
            comments.push(PdfCommentModel {
                element_id: ids::annot_id(&object_key, SCHEMA_VERSION),
                page: number,
                subtype,
                contents,
                author,
            });
        }
    }
    Ok(comments)
}

fn walk_field_list(
    document: &Document,
    list: &Object,
    prefix: &str,
    fields: &mut Vec<PdfFieldModel>,
) -> Result<()> {
    let Object::Array(items) = dereference(document, list)? else {
        return Ok(());
    };
    for item in items {
        walk_field(document, item, prefix, fields)?;
    }
    Ok(())
}

fn walk_field(
    document: &Document,
    object: &Object,
    prefix: &str,
    fields: &mut Vec<PdfFieldModel>,
) -> Result<()> {
    let Some(dict) = resolve_dict(document, object)? else {
        return Ok(());
    };
    let local_name = dict
        .get(b"T")
        .ok()
        .and_then(object_string)
        .unwrap_or_default();
    let name = if prefix.is_empty() {
        local_name.clone()
    } else if local_name.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}.{local_name}")
    };
    let field_type = dict
        .get(b"FT")
        .ok()
        .and_then(object_name)
        .map(normalize_field_type);
    let value = dict
        .get(b"V")
        .ok()
        .and_then(object_string)
        .unwrap_or_default();
    let flags = dict
        .get(b"Ff")
        .ok()
        .and_then(|object| object.as_i64().ok())
        .unwrap_or(0);
    let read_only = flags & 1 == 1;
    let required = flags & 2 == 2;
    let multiline = flags & 4096 == 4096;
    let password = flags & 8192 == 8192;
    let comb = flags & 16_777_216 == 16_777_216;
    let do_not_scroll = flags & 8_388_608 == 8_388_608;
    let do_not_spell_check = flags & 4_194_304 == 4_194_304;
    let rich_text = flags & 33_554_432 == 33_554_432;
    let no_export = flags & 8 == 8;
    let multi_select = flags & 1_048_576 == 1_048_576;
    let combo = flags & 131_072 == 131_072;
    let edit = flags & 262_144 == 262_144;
    let max_length = dict
        .get(b"MaxLen")
        .ok()
        .and_then(|object| object.as_i64().ok())
        .filter(|value| *value > 0)
        .and_then(|value| u32::try_from(value).ok());
    let export_values = collect_export_values(document, dict);
    let options = collect_choice_options(document, dict);
    if let Some(field_type) = field_type.clone() {
        fields.push(PdfFieldModel {
            element_id: ids::field_id(&name, SCHEMA_VERSION),
            name: name.clone(),
            field_type,
            value,
            export_values,
            options,
            page: None,
            read_only,
            required,
            multiline,
            password,
            comb,
            do_not_scroll,
            do_not_spell_check,
            rich_text,
            no_export,
            multi_select,
            combo,
            edit,
            max_length,
        });
    }
    if let Ok(kids) = dict.get(b"Kids") {
        walk_field_list(document, kids, &name, fields)?;
    }
    let _ = field_type;
    Ok(())
}

fn collect_export_values(document: &Document, dict: &lopdf::Dictionary) -> Vec<String> {
    let mut values = Vec::new();
    append_ap_n_keys(document, dict, &mut values);
    if let Ok(kids) = dict.get(b"Kids") {
        let Ok(Object::Array(items)) = dereference(document, kids) else {
            return finalize_export_values(values);
        };
        for item in items {
            if let Ok(Some(kid)) = resolve_dict(document, item) {
                append_ap_n_keys(document, kid, &mut values);
            }
        }
    }
    finalize_export_values(values)
}

fn append_ap_n_keys(document: &Document, dict: &lopdf::Dictionary, values: &mut Vec<String>) {
    let Ok(ap) = dict.get(b"AP") else {
        return;
    };
    let Some(ap_dict) = resolve_dict(document, ap).ok().flatten() else {
        return;
    };
    let Ok(normal) = ap_dict.get(b"N") else {
        return;
    };
    let Some(normal_dict) = resolve_dict(document, normal).ok().flatten() else {
        return;
    };
    for (key, _) in normal_dict.iter() {
        let name = String::from_utf8_lossy(key).into_owned();
        if !values.iter().any(|existing| existing == &name) {
            values.push(name);
        }
    }
}

fn finalize_export_values(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn collect_choice_options(document: &Document, dict: &lopdf::Dictionary) -> Vec<String> {
    let Ok(opt) = dict.get(b"Opt") else {
        return Vec::new();
    };
    let Ok(Object::Array(items)) = dereference(document, opt) else {
        return Vec::new();
    };
    let mut options = Vec::new();
    for item in items {
        let Ok(resolved) = dereference(document, item) else {
            continue;
        };
        match resolved {
            Object::String(_, _) | Object::Name(_) => {
                if let Some(value) = object_string(resolved) {
                    options.push(value);
                }
            }
            Object::Array(pair) => {
                // `/Opt` entries may be `[export display]`; agents set the export value.
                if let Some(first) = pair.first()
                    && let Ok(export_obj) = dereference(document, first)
                    && let Some(export) = object_string(export_obj)
                {
                    options.push(export);
                }
            }
            _ => {}
        }
    }
    options
}

fn collect_outline(document: &Document) -> Vec<String> {
    let mut titles = Vec::new();
    let Ok(catalog) = document.catalog() else {
        return titles;
    };
    let Ok(outlines) = catalog.get(b"Outlines") else {
        return titles;
    };
    let Some(dict) = resolve_dict(document, outlines).ok().flatten() else {
        return titles;
    };
    if let Ok(first) = dict.get(b"First") {
        collect_outline_node(document, first, &mut titles);
    }
    titles
}

fn collect_outline_node(document: &Document, object: &Object, titles: &mut Vec<String>) {
    let Some(dict) = resolve_dict(document, object).ok().flatten() else {
        return;
    };
    if let Some(title) = dict.get(b"Title").ok().and_then(object_string) {
        titles.push(title);
    }
    if let Ok(next) = dict.get(b"Next") {
        collect_outline_node(document, next, titles);
    }
}

fn resolve_dict<'a>(
    document: &'a Document,
    object: &'a Object,
) -> Result<Option<&'a lopdf::Dictionary>> {
    match dereference(document, object)? {
        Object::Dictionary(dict) => Ok(Some(dict)),
        _ => Ok(None),
    }
}

fn dereference<'a>(document: &'a Document, object: &'a Object) -> Result<&'a Object> {
    match object {
        Object::Reference(id) => document
            .get_object(*id)
            .map_err(|error| format_error(format!("missing PDF object: {error}"))),
        other => Ok(other),
    }
}

fn object_string(object: &Object) -> Option<String> {
    match object {
        Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Object::Name(name) => Some(String::from_utf8_lossy(name).into_owned()),
        _ => None,
    }
}

fn object_name(object: &Object) -> Option<String> {
    object
        .as_name()
        .ok()
        .map(|name| String::from_utf8_lossy(name).into_owned())
}

fn normalize_field_type(raw: String) -> String {
    match raw.as_str() {
        "Tx" => "tx".into(),
        "Btn" => "btn".into(),
        "Ch" => "ch".into(),
        "Sig" => "sig".into(),
        other => other.to_ascii_lowercase(),
    }
}

fn format_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<pdf>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_pdf_bytes;
    use crate::fixture::minimal_form_pdf;

    #[test]
    fn parses_form_field_and_page_count() {
        let model = parse_pdf_bytes(&minimal_form_pdf()).expect("parse");
        assert_eq!(model.page_count, 1);
        assert_eq!(model.fields[0].name, "Name");
        assert_eq!(model.fields[0].value, "Ada");
        assert!(!model.encrypted);
    }
}
