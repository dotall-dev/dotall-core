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
        "insert_paragraph" => validate_insert_paragraph(model, operation),
        "delete_paragraph" => validate_delete_paragraph(model, operation),
        "set_paragraph_style" => validate_set_paragraph_style(model, operation),
        "set_paragraph_alignment" => validate_set_paragraph_alignment(model, operation),
        "set_header_paragraph_text" => {
            validate_set_header_footer_text(model, operation, StoryKind::Header)
        }
        "set_footer_paragraph_text" => {
            validate_set_header_footer_text(model, operation, StoryKind::Footer)
        }
        other => Err(format_error(format!(
            "unsupported docx edit `{other}`; use set_paragraph_text, insert_paragraph, delete_paragraph, set_paragraph_style, set_paragraph_alignment, set_header_paragraph_text, or set_footer_paragraph_text"
        ))),
    }
}

fn validate_set_paragraph_text(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let text_payload = resolve_text_payload(&operation.payload)?;
    if !paragraph.editable {
        return Err(format_error(
            "paragraph contains tracked changes, a content control, or a field",
        ));
    }
    let mut payload = serde_json::json!({
        "index": paragraph.index,
        "text": text_payload.text,
        "element_id": paragraph.element_id,
    });
    if let Some(runs) = &text_payload.runs {
        payload["runs"] = serde_json::Value::Array(
            runs.iter()
                .map(|run| serde_json::json!({ "text": run }))
                .collect(),
        );
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_paragraph_text".into(),
            payload,
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_text".into(),
            before: Some(paragraph.text.clone()),
            after: Some(text_payload.text),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_insert_paragraph(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let after = operation
        .payload
        .get("after")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format_error("`after` is required"))? as u32;
    let text = required_str(&operation.payload, "text")?.to_owned();
    let anchor = model
        .paragraphs
        .iter()
        .find(|paragraph| paragraph.index == after)
        .ok_or_else(|| format_error(format!("paragraph `{after}` was not found")))?;
    let new_index = after.saturating_add(1);
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "insert_paragraph".into(),
            payload: serde_json::json!({
                "after": after,
                "text": text,
                "element_id": anchor.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{new_index}"),
            element_id: String::new(),
            change: "insert_paragraph".into(),
            before: None,
            after: Some(text),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_delete_paragraph(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "delete_paragraph".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "delete_paragraph".into(),
            before: Some(paragraph.text.clone()),
            after: None,
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_paragraph_style(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let style_id = required_str(&operation.payload, "style_id")?.to_owned();
    if style_id.trim().is_empty() {
        return Err(format_error("`style_id` must be non-empty"));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_paragraph_style".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "style_id": style_id,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_style".into(),
            before: paragraph.style_id.clone(),
            after: Some(style_id),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_paragraph_alignment(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let alignment = normalize_alignment(required_str(&operation.payload, "alignment")?)?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_paragraph_alignment".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "alignment": alignment,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_alignment".into(),
            before: None,
            after: Some(alignment),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn normalize_alignment(raw: &str) -> Result<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "left" | "start" => Ok("left".into()),
        "center" => Ok("center".into()),
        "right" | "end" => Ok("right".into()),
        "both" | "justify" => Ok("both".into()),
        other => Err(format_error(format!(
            "unsupported alignment `{other}`; use left, center, right, or both"
        ))),
    }
}

fn validate_set_header_footer_text(
    model: &DocumentModel,
    operation: &SemanticOperation,
    kind: StoryKind,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_header_footer_paragraph(model, &operation.payload, kind)?;
    let text_payload = resolve_text_payload(&operation.payload)?;
    if !paragraph.editable {
        return Err(format_error(
            "paragraph contains tracked changes, a content control, or a field",
        ));
    }
    let op_kind = kind.edit_kind();
    let mut payload = serde_json::json!({
        "part": paragraph.part,
        "index": paragraph.index,
        "text": text_payload.text,
        "element_id": paragraph.element_id,
    });
    if let Some(runs) = &text_payload.runs {
        payload["runs"] = serde_json::Value::Array(
            runs.iter()
                .map(|run| serde_json::json!({ "text": run }))
                .collect(),
        );
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: op_kind.into(),
            payload,
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}:{}:{}", kind.prefix(), paragraph.part, paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: op_kind.into(),
            before: Some(paragraph.text.clone()),
            after: Some(text_payload.text),
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
            let text_payload = resolve_text_payload(&operation.payload)?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_text(
                &original,
                index,
                &text_payload.text,
                text_payload.runs.as_deref(),
            )?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "insert_paragraph" => {
            let after = operation
                .payload
                .get("after")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`after` is required"))? as u32;
            let text = required_str(&operation.payload, "text")?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = insert_paragraph_after(&original, after, text)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "delete_paragraph" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = delete_paragraph_at(&original, index)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_style" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let style_id = required_str(&operation.payload, "style_id")?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_style(&original, index, style_id)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_alignment" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let alignment = required_str(&operation.payload, "alignment")?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_alignment(&original, index, alignment)?;
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
            let text_payload = resolve_text_payload(&operation.payload)?;
            let entry_name = format!("word/{part}.xml");
            let original = entry_bytes(package, &entry_name)?;
            let patched_xml = patch_paragraph_text(
                &original,
                index,
                &text_payload.text,
                text_payload.runs.as_deref(),
            )?;
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

pub fn insert_paragraph_after(xml: &[u8], after: u32, text: &str) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("document XML is not UTF-8: {error}")))?;
    let spans = paragraph_spans(source)?;
    let span = spans
        .get(after as usize)
        .ok_or_else(|| format_error(format!("paragraph `{after}` was not found")))?;
    let mut insertion = String::from("<w:p><w:r><w:t xml:space=\"preserve\">");
    insertion.push_str(&xml_escape(text));
    insertion.push_str("</w:t></w:r></w:p>");
    let mut output = String::new();
    output.push_str(&source[..span.1]);
    output.push_str(&insertion);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

pub fn delete_paragraph_at(xml: &[u8], index: u32) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("document XML is not UTF-8: {error}")))?;
    let spans = paragraph_spans(source)?;
    let span = spans
        .get(index as usize)
        .ok_or_else(|| format_error(format!("paragraph `{index}` was not found")))?;
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

pub fn patch_paragraph_style(xml: &[u8], index: u32, style_id: &str) -> Result<Vec<u8>> {
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
    let style_tag = format!(r#"<w:pStyle w:val="{}"/>"#, xml_escape_attr(style_id));
    let replacement = match extract_p_pr(&paragraph[open_end..]) {
        Some(p_pr) => {
            let patched_p_pr = upsert_p_style(p_pr, &style_tag)?;
            format!(
                "{}{}{}",
                &paragraph[..open_end],
                patched_p_pr,
                &paragraph[open_end + p_pr.len()..]
            )
        }
        None => {
            format!(
                "{}<w:pPr>{}</w:pPr>{}",
                &paragraph[..open_end],
                style_tag,
                &paragraph[open_end..]
            )
        }
    };
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&replacement);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

pub fn patch_paragraph_alignment(xml: &[u8], index: u32, alignment: &str) -> Result<Vec<u8>> {
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
    let jc_tag = format!(r#"<w:jc w:val="{}"/>"#, xml_escape_attr(alignment));
    let replacement = match extract_p_pr(&paragraph[open_end..]) {
        Some(p_pr) => {
            let patched_p_pr = upsert_p_jc(p_pr, &jc_tag)?;
            format!(
                "{}{}{}",
                &paragraph[..open_end],
                patched_p_pr,
                &paragraph[open_end + p_pr.len()..]
            )
        }
        None => {
            format!(
                "{}<w:pPr>{}</w:pPr>{}",
                &paragraph[..open_end],
                jc_tag,
                &paragraph[open_end..]
            )
        }
    };
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&replacement);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

fn upsert_p_style(p_pr: &str, style_tag: &str) -> Result<String> {
    if let Some(start) = find_named_open(p_pr, "w:pStyle") {
        let end = element_end(p_pr, start, "w:pStyle")?;
        return Ok(format!("{}{}{}", &p_pr[..start], style_tag, &p_pr[end..]));
    }
    let open_end = p_pr
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:pPr"))?;
    if p_pr[..open_end].ends_with("/>") {
        let open = p_pr[..open_end].trim_end_matches("/>");
        return Ok(format!("{open}>{style_tag}</w:pPr>"));
    }
    Ok(format!(
        "{}{}{}",
        &p_pr[..open_end],
        style_tag,
        &p_pr[open_end..]
    ))
}

fn upsert_p_jc(p_pr: &str, jc_tag: &str) -> Result<String> {
    if let Some(start) = find_named_open(p_pr, "w:jc") {
        let end = element_end(p_pr, start, "w:jc")?;
        return Ok(format!("{}{}{}", &p_pr[..start], jc_tag, &p_pr[end..]));
    }
    let open_end = p_pr
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:pPr"))?;
    if p_pr[..open_end].ends_with("/>") {
        let open = p_pr[..open_end].trim_end_matches("/>");
        return Ok(format!("{open}>{jc_tag}</w:pPr>"));
    }
    Ok(format!(
        "{}{}{}",
        &p_pr[..open_end],
        jc_tag,
        &p_pr[open_end..]
    ))
}

fn find_named_open(xml: &str, tag: &str) -> Option<usize> {
    let needle = format!("<{tag}");
    let mut search = 0;
    while let Some(rel) = xml[search..].find(&needle) {
        let at = search + rel;
        let after = xml.as_bytes().get(at + needle.len()).copied().unwrap_or(0);
        if after == b' ' || after == b'>' || after == b'/' {
            return Some(at);
        }
        search = at + needle.len();
    }
    None
}

fn xml_escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn patch_paragraph_text(
    xml: &[u8],
    index: u32,
    text: &str,
    runs: Option<&[String]>,
) -> Result<Vec<u8>> {
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
    let run_props = extract_run_r_prs(paragraph);
    let run_texts: Vec<&str> = match runs {
        Some(runs) if !runs.is_empty() => runs.iter().map(String::as_str).collect(),
        _ => vec![text],
    };
    let mut replacement = String::new();
    replacement.push_str(&paragraph[..open_end]);
    if let Some(p_pr) = p_pr {
        replacement.push_str(p_pr);
    }
    for (run_index, run_text) in run_texts.iter().enumerate() {
        replacement.push_str("<w:r>");
        let r_pr = if runs.is_some() {
            // Explicit runs: clone matching run rPr when present; extras reuse first-run rPr.
            run_props
                .get(run_index)
                .copied()
                .flatten()
                .or_else(|| run_props.first().copied().flatten())
        } else {
            // Plain text: preserve first text-run rPr and clear subsequent runs.
            run_props.first().copied().flatten()
        };
        if let Some(r_pr) = r_pr {
            replacement.push_str(r_pr);
        }
        replacement.push_str("<w:t xml:space=\"preserve\">");
        replacement.push_str(&xml_escape(run_text));
        replacement.push_str("</w:t></w:r>");
    }
    replacement.push_str("</w:p>");
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

/// Collect `w:rPr` from each direct `w:r` child (skips paragraph-mark rPr inside `w:pPr`).
fn extract_run_r_prs(paragraph: &str) -> Vec<Option<&str>> {
    let mut props = Vec::new();
    let mut cursor = 0;
    while cursor < paragraph.len() {
        let rest = &paragraph[cursor..];
        let Some(rel) = find_run_open(rest) else {
            break;
        };
        let start = cursor + rel;
        let Ok(end) = element_end(paragraph, start, "w:r") else {
            break;
        };
        let run = &paragraph[start..end];
        props.push(extract_r_pr_in_run(run));
        cursor = end;
    }
    props
}

fn find_run_open(xml: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(rel) = xml[search..].find("<w:r") {
        let at = search + rel;
        let after = xml.as_bytes().get(at + 4).copied().unwrap_or(0);
        // Match <w:r …> / <w:r/> but not <w:rPr …> or <w:rFonts …>.
        if after == b' ' || after == b'>' || after == b'/' {
            return Some(at);
        }
        search = at + 4;
    }
    None
}

fn element_end(xml: &str, start: usize, local: &str) -> Result<usize> {
    let open = format!("<{local}");
    let close = format!("</{local}>");
    let rest = &xml[start..];
    if !rest.starts_with(&open) {
        return Err(format_error(format!("expected `{local}` open tag")));
    }
    let gt = rest
        .find('>')
        .ok_or_else(|| format_error(format!("unterminated `{local}`")))?;
    if rest.as_bytes().get(gt.saturating_sub(1)) == Some(&b'/') {
        return Ok(start + gt + 1);
    }
    rest.find(&close)
        .map(|rel| start + rel + close.len())
        .ok_or_else(|| format_error(format!("unterminated `{local}`")))
}

fn extract_r_pr_in_run(run: &str) -> Option<&str> {
    let open_end = run.find('>')? + 1;
    let inner = &run[open_end..];
    let start = inner.find("<w:rPr")?;
    let abs = open_end + start;
    if let Some(rel) = run[abs..].find("/>")
        && !run[abs..abs + rel].contains('>')
    {
        return Some(&run[abs..abs + rel + 2]);
    }
    run[abs..]
        .find("</w:rPr>")
        .map(|rel| &run[abs..abs + rel + "</w:rPr>".len()])
}

struct TextPayload {
    text: String,
    runs: Option<Vec<String>>,
}

fn resolve_text_payload(payload: &serde_json::Value) -> Result<TextPayload> {
    let runs = match payload.get("runs") {
        Some(serde_json::Value::Array(items)) => {
            if items.is_empty() {
                return Err(format_error("`runs` must be a non-empty array"));
            }
            let mut texts = Vec::with_capacity(items.len());
            for item in items {
                let text = item
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format_error("each run requires `text`"))?;
                texts.push(text.to_owned());
            }
            Some(texts)
        }
        Some(_) => return Err(format_error("`runs` must be an array of `{text}` objects")),
        None => None,
    };
    let text = if let Some(runs) = &runs {
        let joined = runs.concat();
        if let Some(explicit) = payload.get("text").and_then(serde_json::Value::as_str)
            && explicit != joined
        {
            return Err(format_error(
                "`text` must equal the concatenation of `runs` when both are provided",
            ));
        }
        joined
    } else {
        required_str(payload, "text")?.to_owned()
    };
    Ok(TextPayload { text, runs })
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
