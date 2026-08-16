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
use crate::model::{DocumentModel, HeaderFooterParagraphModel, SCHEMA_ID, SCHEMA_VERSION};

const UNSAFE_MARKERS: [&str; 5] = ["<w:del", "<w:ins", "<w:sdt", "<w:fldChar", "<w:instrText"];

pub fn validate(model: &DocumentModel, operations: &[SemanticOperation]) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "docx supports a single paragraph edit operation per transaction",
        ));
    }
    let operation = &operations[0];
    match operation.kind.as_str() {
        "set_paragraph_text" => validate_set_paragraph_text(model, operation),
        "set_header_paragraph_text" => {
            validate_set_header_footer_text(model, operation, StoryKind::Header)
        }
        "set_footer_paragraph_text" => {
            validate_set_header_footer_text(model, operation, StoryKind::Footer)
        }
        other => Err(format_error(format!(
            "unsupported docx edit `{other}`; use set_paragraph_text, set_header_paragraph_text, or set_footer_paragraph_text"
        ))),
    }
}

fn validate_set_paragraph_text(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let text = required_str(&operation.payload, "text")?;
    if !paragraph.editable {
        return Err(format_error(
            "paragraph contains tracked changes, a content control, or a field",
        ));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_paragraph_text".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "text": text,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_text".into(),
            before: Some(paragraph.text.clone()),
            after: Some(text.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_header_footer_text(
    model: &DocumentModel,
    operation: &SemanticOperation,
    kind: StoryKind,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_header_footer_paragraph(model, &operation.payload, kind)?;
    let text = required_str(&operation.payload, "text")?;
    if !paragraph.editable {
        return Err(format_error(
            "paragraph contains tracked changes, a content control, or a field",
        ));
    }
    let op_kind = kind.edit_kind();
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: op_kind.into(),
            payload: serde_json::json!({
                "part": paragraph.part,
                "index": paragraph.index,
                "text": text,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}:{}:{}", kind.prefix(), paragraph.part, paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: op_kind.into(),
            before: Some(paragraph.text.clone()),
            after: Some(text.to_owned()),
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
    match operation.kind.as_str() {
        "set_paragraph_text" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let text = required_str(&operation.payload, "text")?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_text(&original, index, text)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_header_paragraph_text" | "set_footer_paragraph_text" => {
            let part = required_str(&operation.payload, "part")?;
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let text = required_str(&operation.payload, "text")?;
            let entry_name = format!("word/{part}.xml");
            let original = entry_bytes(package, &entry_name)?;
            let patched_xml = patch_paragraph_text(&original, index, text)?;
            let bytes = rebuild_package(package, &BTreeMap::from([(entry_name, patched_xml)]))?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        other => Err(format_error(format!(
            "unsupported validated edit `{other}`"
        ))),
    }
}

pub fn patch_paragraph_text(xml: &[u8], index: u32, text: &str) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("document XML is not UTF-8: {error}")))?;
    let spans = paragraph_spans(source)?;
    let span = spans
        .get(index as usize)
        .ok_or_else(|| format_error(format!("paragraph `{index}` was not found")))?;
    let paragraph = &source[span.0..span.1];
    if UNSAFE_MARKERS
        .iter()
        .any(|marker| paragraph.contains(marker))
    {
        return Err(format_error(
            "paragraph contains tracked changes, a content control, or a field",
        ));
    }
    let open_end = paragraph
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:p"))?;
    let p_pr = extract_p_pr(&paragraph[open_end..]);
    let r_pr = extract_first_r_pr(paragraph);
    let mut replacement = String::new();
    replacement.push_str(&paragraph[..open_end]);
    if let Some(p_pr) = p_pr {
        replacement.push_str(p_pr);
    }
    replacement.push_str("<w:r>");
    if let Some(r_pr) = r_pr {
        replacement.push_str(r_pr);
    }
    replacement.push_str("<w:t xml:space=\"preserve\">");
    replacement.push_str(&xml_escape(text));
    replacement.push_str("</w:t></w:r></w:p>");
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&replacement);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

#[derive(Clone, Copy)]
enum StoryKind {
    Header,
    Footer,
}

impl StoryKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::Footer => "footer",
        }
    }

    fn edit_kind(self) -> &'static str {
        match self {
            Self::Header => "set_header_paragraph_text",
            Self::Footer => "set_footer_paragraph_text",
        }
    }

    fn paragraphs(self, model: &DocumentModel) -> &[HeaderFooterParagraphModel] {
        match self {
            Self::Header => &model.header_paragraphs,
            Self::Footer => &model.footer_paragraphs,
        }
    }
}

fn resolve_body_paragraph<'a>(
    model: &'a DocumentModel,
    payload: &serde_json::Value,
) -> Result<&'a crate::model::ParagraphModel> {
    if let Some(element_id) = payload
        .get("element_id")
        .and_then(serde_json::Value::as_str)
    {
        return model
            .paragraphs
            .iter()
            .find(|paragraph| paragraph.element_id == element_id)
            .ok_or_else(|| format_error(format!("paragraph `{element_id}` was not found")));
    }
    let index = payload
        .get("index")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format_error("`index` or `element_id` is required"))? as u32;
    model
        .paragraphs
        .iter()
        .find(|paragraph| paragraph.index == index)
        .ok_or_else(|| format_error(format!("paragraph `{index}` was not found")))
}

fn resolve_header_footer_paragraph<'a>(
    model: &'a DocumentModel,
    payload: &serde_json::Value,
    kind: StoryKind,
) -> Result<&'a HeaderFooterParagraphModel> {
    let paragraphs = kind.paragraphs(model);
    if let Some(element_id) = payload
        .get("element_id")
        .and_then(serde_json::Value::as_str)
    {
        return paragraphs
            .iter()
            .find(|paragraph| paragraph.element_id == element_id)
            .ok_or_else(|| {
                format_error(format!(
                    "{} paragraph `{element_id}` was not found",
                    kind.prefix()
                ))
            });
    }
    let index = payload
        .get("index")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format_error("`index` or `element_id` is required"))? as u32;
    if let Some(part) = payload.get("part").and_then(serde_json::Value::as_str) {
        return paragraphs
            .iter()
            .find(|paragraph| paragraph.part == part && paragraph.index == index)
            .ok_or_else(|| {
                format_error(format!(
                    "{} paragraph `{part}:{index}` was not found",
                    kind.prefix()
                ))
            });
    }
    // Bare index: first match in sorted part order (model already sorted).
    paragraphs.get(index as usize).ok_or_else(|| {
        format_error(format!(
            "{} paragraph `{index}` was not found",
            kind.prefix()
        ))
    })
}

fn paragraph_spans(xml: &str) -> Result<Vec<(usize, usize)>> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    while cursor < xml.len() {
        let rest = &xml[cursor..];
        let Some(rel) = find_paragraph_open(rest) else {
            break;
        };
        let start = cursor + rel;
        let end = paragraph_end(xml, start)?;
        spans.push((start, end));
        cursor = end;
    }
    Ok(spans)
}

fn find_paragraph_open(xml: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(rel) = xml[search..].find("<w:p") {
        let at = search + rel;
        let after = xml.as_bytes().get(at + 4).copied().unwrap_or(0);
        if after == b' ' || after == b'>' || after == b'/' {
            return Some(at);
        }
        search = at + 4;
    }
    None
}

fn paragraph_end(xml: &str, start: usize) -> Result<usize> {
    let rest = &xml[start..];
    let gt = rest
        .find('>')
        .ok_or_else(|| format_error("unterminated w:p"))?;
    if rest.as_bytes().get(gt.saturating_sub(1)) == Some(&b'/') {
        return Ok(start + gt + 1);
    }
    rest.find("</w:p>")
        .map(|rel| start + rel + "</w:p>".len())
        .ok_or_else(|| format_error("unterminated w:p"))
}

fn extract_p_pr(inner: &str) -> Option<&str> {
    let start = inner.find("<w:pPr")?;
    if let Some(rel) = inner[start..].find("/>")
        && !inner[start..start + rel].contains('>')
    {
        return Some(&inner[start..start + rel + 2]);
    }
    inner[start..]
        .find("</w:pPr>")
        .map(|rel| &inner[start..start + rel + "</w:pPr>".len()])
}

fn extract_first_r_pr(paragraph: &str) -> Option<&str> {
    let start = paragraph.find("<w:rPr")?;
    if let Some(rel) = paragraph[start..].find("/>")
        && !paragraph[start..start + rel].contains('>')
    {
        return Some(&paragraph[start..start + rel + 2]);
    }
    paragraph[start..]
        .find("</w:rPr>")
        .map(|rel| &paragraph[start..start + rel + "</w:rPr>".len()])
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn entry_bytes(package: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(format!("invalid DOCX package: {error}")))?;
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
        .map_err(|error| format_error(format!("invalid DOCX package: {error}")))?;
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
        .map_err(|error| format_error(format!("cannot finish DOCX package: {error}")))
        .map(|cursor| cursor.into_inner())
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
        path: "<docx edit>".into(),
        message: message.into(),
    }
}
