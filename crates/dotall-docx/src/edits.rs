use std::collections::{BTreeMap, BTreeSet};
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
const HYPERLINK_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const NUMBERING_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
const COMMENTS_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
const IMAGE_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const DOCUMENT_RELS_NAME: &str = "word/_rels/document.xml.rels";
const NUMBERING_PART: &str = "word/numbering.xml";
const COMMENTS_PART: &str = "word/comments.xml";
const CONTENT_TYPES_NAME: &str = "[Content_Types].xml";
const NUMBERING_OVERRIDE: &str = concat!(
    r#"<Override PartName="/word/numbering.xml" ContentType=""#,
    "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml",
    r#""/>"#
);
const COMMENTS_OVERRIDE: &str = concat!(
    r#"<Override PartName="/word/comments.xml" ContentType=""#,
    "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml",
    r#""/>"#
);
const PNG_DEFAULT: &str = r#"<Default Extension="png" ContentType="image/png"/>"#;
const JPEG_DEFAULT: &str = r#"<Default Extension="jpeg" ContentType="image/jpeg"/>"#;
const JPG_DEFAULT: &str = r#"<Default Extension="jpg" ContentType="image/jpeg"/>"#;
const MINIMAL_BULLET_NUMBERING: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="&#8226;"/><w:lvlJc w:val="left"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#;
const EMPTY_COMMENTS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"></w:comments>"#;
const R_NAMESPACE: &str =
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const EMPTY_RELATIONSHIPS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>"#;

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
        "set_paragraph_spacing" => validate_set_paragraph_spacing(model, operation),
        "insert_page_break" => validate_insert_page_break(model, operation),
        "set_paragraph_bold" => validate_set_paragraph_bold(model, operation),
        "set_paragraph_italic" => validate_set_paragraph_italic(model, operation),
        "set_paragraph_underline" => validate_set_paragraph_underline(model, operation),
        "set_paragraph_font_size" => validate_set_paragraph_font_size(model, operation),
        "set_paragraph_font_name" => validate_set_paragraph_font_name(model, operation),
        "set_paragraph_font_color" => validate_set_paragraph_font_color(model, operation),
        "set_paragraph_highlight" => validate_set_paragraph_highlight(model, operation),
        "set_paragraph_strikethrough" => validate_set_paragraph_strikethrough(model, operation),
        "set_paragraph_vert_align" => validate_set_paragraph_vert_align(model, operation),
        "set_paragraph_caps" => validate_set_paragraph_caps(model, operation),
        "set_paragraph_hyperlink" => validate_set_paragraph_hyperlink(model, operation),
        "set_paragraph_bullet" => validate_set_paragraph_bullet(model, operation),
        "set_cell_shading" => validate_set_cell_shading(model, operation),
        "replace_paragraph_text" => validate_replace_paragraph_text(model, operation),
        "replace_across_paragraphs" => validate_replace_across_paragraphs(model, operation),
        "insert_comment" => validate_insert_comment(model, operation),
        "insert_picture" => validate_insert_picture(model, operation),
        "set_header_paragraph_text" => {
            validate_set_header_footer_text(model, operation, StoryKind::Header)
        }
        "set_footer_paragraph_text" => {
            validate_set_header_footer_text(model, operation, StoryKind::Footer)
        }
        other => Err(format_error(format!(
            "unsupported docx edit `{other}`; use set_paragraph_text, replace_paragraph_text, replace_across_paragraphs, insert_paragraph, delete_paragraph, set_paragraph_style, set_paragraph_alignment, set_paragraph_spacing, insert_page_break, set_paragraph_bold, set_paragraph_italic, set_paragraph_underline, set_paragraph_font_size, set_paragraph_font_name, set_paragraph_font_color, set_paragraph_highlight, set_paragraph_strikethrough, set_paragraph_vert_align, set_paragraph_caps, set_paragraph_hyperlink, set_paragraph_bullet, set_cell_shading, insert_comment, insert_picture, set_header_paragraph_text, or set_footer_paragraph_text"
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

fn validate_replace_paragraph_text(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let find = required_str(&operation.payload, "find")?;
    let replace = required_str(&operation.payload, "replace")?;
    if find.is_empty() {
        return Err(format_error(
            "replace_paragraph_text requires a non-empty `find` string",
        ));
    }
    if !paragraph.editable {
        return Err(format_error(
            "paragraph contains tracked changes, a content control, or a field",
        ));
    }
    if !paragraph.text.contains(find) {
        return Err(format_error(format!(
            "`find` string was not found in paragraph `{}` text",
            paragraph.index
        )));
    }
    let after_text = paragraph.text.replace(find, replace);
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "replace_paragraph_text".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "find": find,
                "replace": replace,
                "text": after_text,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "replace_paragraph_text".into(),
            before: Some(paragraph.text.clone()),
            after: Some(after_text),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_replace_across_paragraphs(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let find = required_str(&operation.payload, "find")?;
    let replace = required_str(&operation.payload, "replace")?;
    if find.is_empty() {
        return Err(format_error(
            "replace_across_paragraphs requires a non-empty `find` string",
        ));
    }
    let matches: Vec<&crate::model::ParagraphModel> = model
        .paragraphs
        .iter()
        .filter(|paragraph| paragraph.editable && paragraph.text.contains(find))
        .collect();
    if matches.is_empty() {
        return Err(format_error(
            "`find` string was not found in any editable body/table paragraph",
        ));
    }
    let payload_matches: Vec<serde_json::Value> = matches
        .iter()
        .map(|paragraph| {
            serde_json::json!({
                "index": paragraph.index,
                "element_id": paragraph.element_id,
                "text": paragraph.text.replace(find, replace),
            })
        })
        .collect();
    let semantic_diff = matches
        .iter()
        .map(|paragraph| SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "replace_across_paragraphs".into(),
            before: Some(paragraph.text.clone()),
            after: Some(paragraph.text.replace(find, replace)),
        })
        .collect();
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "replace_across_paragraphs".into(),
            payload: serde_json::json!({
                "find": find,
                "replace": replace,
                "matches": payload_matches,
            }),
        }],
        semantic_diff,
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

fn validate_set_paragraph_spacing(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    if !paragraph.editable {
        return Err(format_error(
            "paragraph contains tracked changes, a content control, or a field",
        ));
    }
    let before_pt = optional_non_negative_f64(&operation.payload, "before_pt")?;
    let after_pt = optional_non_negative_f64(&operation.payload, "after_pt")?;
    if before_pt.is_none() && after_pt.is_none() {
        return Err(format_error(
            "set_paragraph_spacing requires at least one of `before_pt` or `after_pt`",
        ));
    }
    let before_twips = before_pt.map(pt_to_twips);
    let after_twips = after_pt.map(pt_to_twips);
    let mut payload = serde_json::json!({
        "index": paragraph.index,
        "element_id": paragraph.element_id,
    });
    if let Some(value) = before_pt {
        payload["before_pt"] = serde_json::json!(value);
    }
    if let Some(value) = after_pt {
        payload["after_pt"] = serde_json::json!(value);
    }
    if let Some(value) = before_twips {
        payload["before_twips"] = serde_json::json!(value);
    }
    if let Some(value) = after_twips {
        payload["after_twips"] = serde_json::json!(value);
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_paragraph_spacing".into(),
            payload,
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_spacing".into(),
            before: None,
            after: Some(format!(
                "before_pt={:?}, after_pt={:?}",
                before_pt, after_pt
            )),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_insert_page_break(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
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
            kind: "insert_page_break".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "insert_page_break".into(),
            before: None,
            after: Some("page".into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn pt_to_twips(pt: f64) -> u32 {
    (pt * 20.0).round() as u32
}

fn validate_set_paragraph_bold(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    validate_set_paragraph_run_bool(model, operation, "set_paragraph_bold", "bold")
}

fn validate_set_paragraph_italic(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    validate_set_paragraph_run_bool(model, operation, "set_paragraph_italic", "italic")
}

fn validate_set_paragraph_underline(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    validate_set_paragraph_run_bool(model, operation, "set_paragraph_underline", "underline")
}

fn validate_set_paragraph_strikethrough(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    validate_set_paragraph_run_bool(
        model,
        operation,
        "set_paragraph_strikethrough",
        "strikethrough",
    )
}

fn validate_set_paragraph_vert_align(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let vert_align = optional_vert_align(&operation.payload, "vert_align")?;
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
            kind: "set_paragraph_vert_align".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "vert_align": vert_align,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_vert_align".into(),
            before: None,
            after: Some(match vert_align {
                Some(value) => value.to_owned(),
                None => "cleared".into(),
            }),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_paragraph_caps(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let caps = optional_caps(&operation.payload, "caps")?;
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
            kind: "set_paragraph_caps".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "caps": caps,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_caps".into(),
            before: None,
            after: Some(match caps {
                Some(value) => value.to_owned(),
                None => "cleared".into(),
            }),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_paragraph_bullet(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let bullet = required_bool(&operation.payload, "bullet")?;
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
            kind: "set_paragraph_bullet".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "bullet": bullet,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_bullet".into(),
            before: None,
            after: Some(if bullet { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_insert_comment(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let text = required_str(&operation.payload, "text")?;
    let author = operation
        .payload
        .get("author")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("Dotall");
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
            kind: "insert_comment".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "text": text,
                "author": author,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "insert_comment".into(),
            before: None,
            after: Some(text.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_insert_picture(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    if !paragraph.editable {
        return Err(format_error(
            "paragraph contains tracked changes, a content control, or a field",
        ));
    }
    let (bytes, content_type) = decode_picture_payload(&operation.payload)?;
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "insert_picture".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "bytes_base64": encoded,
                "content_type": content_type,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "insert_picture".into(),
            before: None,
            after: Some(content_type),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn decode_picture_payload(payload: &serde_json::Value) -> Result<(Vec<u8>, String)> {
    use base64::Engine;
    let encoded = required_str(payload, "bytes_base64")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format_error(format!("invalid bytes_base64: {error}")))?;
    if bytes.is_empty() {
        return Err(format_error("image bytes must not be empty"));
    }
    let content_type = payload
        .get("content_type")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("image/png");
    let content_type = match content_type {
        "image/png" | "image/jpeg" => content_type.to_owned(),
        other => {
            return Err(format_error(format!(
                "unsupported content_type `{other}`; use image/png or image/jpeg"
            )));
        }
    };
    match content_type.as_str() {
        "image/png" if !bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) => {
            return Err(format_error("invalid image: PNG signature mismatch"));
        }
        "image/jpeg" if !(bytes.starts_with(&[0xFF, 0xD8])) => {
            return Err(format_error("invalid image: JPEG signature mismatch"));
        }
        _ => {}
    }
    Ok((bytes, content_type))
}

fn validate_set_paragraph_hyperlink(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_hyperlink_paragraph(model, &operation.payload)?;
    let url = optional_hyperlink_url(&operation.payload, "url")?;
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
            kind: "set_paragraph_hyperlink".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "url": url,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_hyperlink".into(),
            before: None,
            after: Some(match url {
                Some(value) => value.to_owned(),
                None => "cleared".into(),
            }),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_paragraph_font_size(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let size_pt = optional_positive_f64(&operation.payload, "size_pt")?;
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
            kind: "set_paragraph_font_size".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "size_pt": size_pt,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_font_size".into(),
            before: None,
            after: Some(match size_pt {
                Some(value) => format_size_pt(value),
                None => "cleared".into(),
            }),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_paragraph_font_name(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let font = optional_str(&operation.payload, "font")?;
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
            kind: "set_paragraph_font_name".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "font": font,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_font_name".into(),
            before: None,
            after: Some(match font {
                Some(value) => value.to_owned(),
                None => "cleared".into(),
            }),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_paragraph_font_color(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let color = optional_srgb_color(&operation.payload, "color")?;
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
            kind: "set_paragraph_font_color".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "color": color,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_font_color".into(),
            before: None,
            after: Some(match color.as_deref() {
                Some(value) => value.to_owned(),
                None => "cleared".into(),
            }),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_cell_shading(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let color = optional_srgb_color(&operation.payload, "color")?;
    if !paragraph.editable {
        return Err(format_error(
            "paragraph contains tracked changes, a content control, or a field",
        ));
    }
    if !paragraph.in_table {
        return Err(format_error("paragraph is not inside a table cell"));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_cell_shading".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "color": color,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_cell_shading".into(),
            before: None,
            after: Some(match color.as_deref() {
                Some(value) => value.to_owned(),
                None => "cleared".into(),
            }),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_paragraph_highlight(
    model: &DocumentModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let color = optional_highlight_color(&operation.payload, "color")?;
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
            kind: "set_paragraph_highlight".into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                "color": color,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: "set_paragraph_highlight".into(),
            before: None,
            after: Some(match color.as_deref() {
                Some(value) => value.to_owned(),
                None => "cleared".into(),
            }),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_paragraph_run_bool(
    model: &DocumentModel,
    operation: &SemanticOperation,
    kind: &str,
    flag: &str,
) -> Result<ValidatedEdit> {
    let paragraph = resolve_body_paragraph(model, &operation.payload)?;
    let enabled = required_bool(&operation.payload, flag)?;
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
            kind: kind.into(),
            payload: serde_json::json!({
                "index": paragraph.index,
                flag: enabled,
                "element_id": paragraph.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("paragraph:{}", paragraph.index),
            element_id: paragraph.element_id.clone(),
            change: kind.into(),
            before: None,
            after: Some(if enabled { "true" } else { "false" }.into()),
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
        "set_paragraph_text" | "replace_paragraph_text" => {
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
        "replace_across_paragraphs" => {
            let matches = operation
                .payload
                .get("matches")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| format_error("`matches` is required"))?;
            let original = entry_bytes(package, "word/document.xml")?;
            let mut patched_xml = original;
            for item in matches {
                let index = item
                    .get("index")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| format_error("`index` is required"))?
                    as u32;
                let text = required_str(item, "text")?;
                patched_xml = patch_paragraph_text(&patched_xml, index, text, None)?;
            }
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
        "set_paragraph_spacing" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let before_twips = operation
                .payload
                .get("before_twips")
                .and_then(serde_json::Value::as_u64)
                .map(|value| value as u32);
            let after_twips = operation
                .payload
                .get("after_twips")
                .and_then(serde_json::Value::as_u64)
                .map(|value| value as u32);
            if before_twips.is_none() && after_twips.is_none() {
                return Err(format_error(
                    "set_paragraph_spacing requires at least one of `before_pt` or `after_pt`",
                ));
            }
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_spacing(&original, index, before_twips, after_twips)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "insert_page_break" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_insert_page_break(&original, index)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_bold" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let bold = operation
                .payload
                .get("bold")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`bold` boolean is required"))?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_run_bool(&original, index, "w:b", bold)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_italic" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let italic = operation
                .payload
                .get("italic")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`italic` boolean is required"))?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_run_bool(&original, index, "w:i", italic)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_underline" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let underline = operation
                .payload
                .get("underline")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`underline` boolean is required"))?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_underline(&original, index, underline)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_font_size" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let size_pt = match operation.payload.get("size_pt") {
                None | Some(serde_json::Value::Null) => None,
                Some(value) => Some(
                    value
                        .as_f64()
                        .filter(|v| v.is_finite() && *v > 0.0)
                        .ok_or_else(|| format_error("`size_pt` must be a positive number"))?,
                ),
            };
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_font_size(&original, index, size_pt)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_font_name" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let font = optional_str(&operation.payload, "font")?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_font_name(&original, index, font)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_font_color" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let color = optional_srgb_color(&operation.payload, "color")?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_font_color(&original, index, color.as_deref())?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_highlight" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let color = optional_highlight_color(&operation.payload, "color")?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_highlight(&original, index, color.as_deref())?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_strikethrough" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let strikethrough = operation
                .payload
                .get("strikethrough")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`strikethrough` boolean is required"))?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_strikethrough(&original, index, strikethrough)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_vert_align" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let vert_align = optional_vert_align(&operation.payload, "vert_align")?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_vert_align(&original, index, vert_align)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_caps" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let caps = optional_caps(&operation.payload, "caps")?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_paragraph_caps(&original, index, caps)?;
            let bytes = rebuild_package(
                package,
                &BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]),
            )?;
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
        "set_paragraph_hyperlink" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let url = optional_hyperlink_url(&operation.payload, "url")?;
            apply_paragraph_hyperlink(package, index, url)
        }
        "set_paragraph_bullet" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let bullet = required_bool(&operation.payload, "bullet")?;
            apply_paragraph_bullet(package, index, bullet)
        }
        "insert_comment" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let text = required_str(&operation.payload, "text")?;
            let author = operation
                .payload
                .get("author")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or("Dotall");
            apply_insert_comment(package, index, text, author)
        }
        "insert_picture" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let (bytes, content_type) = decode_picture_payload(&operation.payload)?;
            apply_insert_picture(package, index, &bytes, &content_type)
        }
        "set_cell_shading" => {
            let index = operation
                .payload
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format_error("`index` is required"))? as u32;
            let color = optional_srgb_color(&operation.payload, "color")?;
            let original = entry_bytes(package, "word/document.xml")?;
            let patched_xml = patch_cell_shading(&original, index, color.as_deref())?;
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

pub fn patch_paragraph_spacing(
    xml: &[u8],
    index: u32,
    before_twips: Option<u32>,
    after_twips: Option<u32>,
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
    let replacement = match extract_p_pr(&paragraph[open_end..]) {
        Some(p_pr) => {
            let patched_p_pr = upsert_p_spacing(p_pr, before_twips, after_twips)?;
            format!(
                "{}{}{}",
                &paragraph[..open_end],
                patched_p_pr,
                &paragraph[open_end + p_pr.len()..]
            )
        }
        None => {
            let spacing_tag = format_spacing_tag(before_twips, after_twips);
            format!(
                "{}<w:pPr>{}</w:pPr>{}",
                &paragraph[..open_end],
                spacing_tag,
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

fn upsert_p_spacing(
    p_pr: &str,
    before_twips: Option<u32>,
    after_twips: Option<u32>,
) -> Result<String> {
    let (existing_before, existing_after) = if let Some(start) = find_named_open(p_pr, "w:spacing")
    {
        let end = element_end(p_pr, start, "w:spacing")?;
        let existing = &p_pr[start..end];
        (
            attr_u32(existing, "w:before"),
            attr_u32(existing, "w:after"),
        )
    } else {
        (None, None)
    };
    let merged_before = before_twips.or(existing_before);
    let merged_after = after_twips.or(existing_after);
    let spacing_tag = format_spacing_tag(merged_before, merged_after);
    if let Some(start) = find_named_open(p_pr, "w:spacing") {
        let end = element_end(p_pr, start, "w:spacing")?;
        return Ok(format!("{}{}{}", &p_pr[..start], spacing_tag, &p_pr[end..]));
    }
    let open_end = p_pr
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:pPr"))?;
    if p_pr[..open_end].ends_with("/>") {
        let open = p_pr[..open_end].trim_end_matches("/>");
        return Ok(format!("{open}>{spacing_tag}</w:pPr>"));
    }
    Ok(format!(
        "{}{}{}",
        &p_pr[..open_end],
        spacing_tag,
        &p_pr[open_end..]
    ))
}

fn format_spacing_tag(before_twips: Option<u32>, after_twips: Option<u32>) -> String {
    let mut tag = String::from("<w:spacing");
    if let Some(before) = before_twips {
        tag.push_str(&format!(r#" w:before="{before}""#));
    }
    if let Some(after) = after_twips {
        tag.push_str(&format!(r#" w:after="{after}""#));
    }
    tag.push_str("/>");
    tag
}

fn attr_u32(element: &str, name: &str) -> Option<u32> {
    let needle = format!(r#"{name}=""#);
    let at = element.find(&needle)?;
    let value_start = at + needle.len();
    let value_end = element[value_start..].find('"')? + value_start;
    element[value_start..value_end].parse().ok()
}

pub fn patch_insert_page_break(xml: &[u8], index: u32) -> Result<Vec<u8>> {
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
    let (content_start, _) = paragraph_content_span(paragraph)?;
    const BREAK_RUN: &str = r#"<w:r><w:br w:type="page"/></w:r>"#;
    let mut replacement = String::new();
    replacement.push_str(&paragraph[..content_start]);
    replacement.push_str(BREAK_RUN);
    replacement.push_str(&paragraph[content_start..]);
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&replacement);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

pub fn patch_paragraph_bold(xml: &[u8], index: u32, bold: bool) -> Result<Vec<u8>> {
    patch_paragraph_run_bool(xml, index, "w:b", bold)
}

pub fn patch_paragraph_italic(xml: &[u8], index: u32, italic: bool) -> Result<Vec<u8>> {
    patch_paragraph_run_bool(xml, index, "w:i", italic)
}

pub fn patch_paragraph_underline(xml: &[u8], index: u32, underline: bool) -> Result<Vec<u8>> {
    let prop = if underline {
        r#"<w:u w:val="single"/>"#
    } else {
        r#"<w:u w:val="none"/>"#
    };
    patch_paragraph_run_prop(xml, index, "w:u", prop)
}

pub fn patch_paragraph_strikethrough(
    xml: &[u8],
    index: u32,
    strikethrough: bool,
) -> Result<Vec<u8>> {
    patch_paragraph_run_bool(xml, index, "w:strike", strikethrough)
}

pub fn patch_paragraph_vert_align(
    xml: &[u8],
    index: u32,
    vert_align: Option<&str>,
) -> Result<Vec<u8>> {
    match vert_align {
        Some("superscript") => {
            let prop = r#"<w:vertAlign w:val="superscript"/>"#;
            patch_paragraph_named_run_prop(xml, index, "w:vertAlign", Some(prop))
        }
        Some("subscript") => {
            let prop = r#"<w:vertAlign w:val="subscript"/>"#;
            patch_paragraph_named_run_prop(xml, index, "w:vertAlign", Some(prop))
        }
        Some(other) => Err(format_error(format!(
            "`vert_align` must be superscript, subscript, or null; got `{other}`"
        ))),
        None => patch_paragraph_named_run_prop(xml, index, "w:vertAlign", None),
    }
}

pub fn patch_paragraph_caps(xml: &[u8], index: u32, caps: Option<&str>) -> Result<Vec<u8>> {
    match caps {
        Some("small") => {
            let cleared = patch_paragraph_named_run_prop(xml, index, "w:caps", None)?;
            patch_paragraph_named_run_prop(
                &cleared,
                index,
                "w:smallCaps",
                Some(r#"<w:smallCaps/>"#),
            )
        }
        Some("all") => {
            let cleared = patch_paragraph_named_run_prop(xml, index, "w:smallCaps", None)?;
            patch_paragraph_named_run_prop(&cleared, index, "w:caps", Some(r#"<w:caps/>"#))
        }
        Some(other) => Err(format_error(format!(
            "`caps` must be small, all, or null; got `{other}`"
        ))),
        None => {
            let cleared = patch_paragraph_named_run_prop(xml, index, "w:smallCaps", None)?;
            patch_paragraph_named_run_prop(&cleared, index, "w:caps", None)
        }
    }
}

fn apply_paragraph_hyperlink(
    package: &[u8],
    index: u32,
    url: Option<&str>,
) -> Result<PatchedOutput> {
    let original = entry_bytes(package, "word/document.xml")?;
    let rels_exist = has_entry(package, DOCUMENT_RELS_NAME)?;
    let rels_xml = if rels_exist {
        entry_bytes(package, DOCUMENT_RELS_NAME)?
    } else {
        EMPTY_RELATIONSHIPS.to_vec()
    };
    let existing_rid = existing_paragraph_hyperlink_rid(&original, index)?;

    let (patched_xml, patched_rels, write_rels) = match url {
        Some(url) => {
            let rid = match existing_rid {
                Some(rid) => rid,
                None => {
                    let used = relationship_id_numbers(&rels_xml)?;
                    format!("rId{}", lowest_unused_number(&used))
                }
            };
            let patched_xml = patch_paragraph_hyperlink(&original, index, Some(&rid))?;
            let patched_rels = upsert_hyperlink_relationship(&rels_xml, &rid, url)?;
            (patched_xml, patched_rels, true)
        }
        None => {
            let patched_xml = patch_paragraph_hyperlink(&original, index, None)?;
            let patched_rels = if let Some(rid) = existing_rid {
                remove_relationship(&rels_xml, &rid)?
            } else {
                rels_xml
            };
            (patched_xml, patched_rels, rels_exist)
        }
    };

    let mut replacements = BTreeMap::from([("word/document.xml".to_owned(), patched_xml)]);
    if write_rels {
        replacements.insert(DOCUMENT_RELS_NAME.to_owned(), patched_rels);
    }
    let bytes = rebuild_package(package, &replacements)?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn apply_paragraph_bullet(package: &[u8], index: u32, bullet: bool) -> Result<PatchedOutput> {
    let original = entry_bytes(package, "word/document.xml")?;
    let mut replacements = BTreeMap::new();
    if bullet {
        let numbering_existed = has_entry(package, NUMBERING_PART)?;
        let (num_id, numbering_patch) = ensure_bullet_numbering(package)?;
        replacements.insert(
            "word/document.xml".to_owned(),
            patch_paragraph_num_pr(&original, index, Some(num_id))?,
        );
        if let Some(numbering) = numbering_patch {
            replacements.insert(NUMBERING_PART.to_owned(), numbering);
        }
        if !numbering_existed {
            ensure_numbering_package_links(package, &mut replacements)?;
        }
    } else {
        replacements.insert(
            "word/document.xml".to_owned(),
            patch_paragraph_num_pr(&original, index, None)?,
        );
    }
    let bytes = rebuild_package(package, &replacements)?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn apply_insert_comment(
    package: &[u8],
    index: u32,
    text: &str,
    author: &str,
) -> Result<PatchedOutput> {
    let comments_existed = has_entry(package, COMMENTS_PART)?;
    let comments_xml = if comments_existed {
        entry_bytes(package, COMMENTS_PART)?
    } else {
        EMPTY_COMMENTS.to_vec()
    };
    let (comment_id, patched_comments) = append_comment_entry(&comments_xml, text, author)?;
    let document = entry_bytes(package, "word/document.xml")?;
    let patched_document = patch_paragraph_comment_markers(&document, index, comment_id)?;

    let mut replacements = BTreeMap::from([
        ("word/document.xml".to_owned(), patched_document),
        (COMMENTS_PART.to_owned(), patched_comments),
    ]);
    if !comments_existed {
        ensure_comments_package_links(package, &mut replacements)?;
    }
    let bytes = rebuild_package(package, &replacements)?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn apply_insert_picture(
    package: &[u8],
    index: u32,
    image_bytes: &[u8],
    content_type: &str,
) -> Result<PatchedOutput> {
    let extension = match content_type {
        "image/png" => "png",
        "image/jpeg" => "jpeg",
        other => {
            return Err(format_error(format!(
                "unsupported content_type `{other}`; use image/png or image/jpeg"
            )));
        }
    };
    let image_n = next_media_image_number(package)?;
    let media_name = format!("word/media/image{image_n}.{extension}");
    let rel_target = format!("media/image{image_n}.{extension}");

    let rels_exist = has_entry(package, DOCUMENT_RELS_NAME)?;
    let rels_xml = if rels_exist {
        entry_bytes(package, DOCUMENT_RELS_NAME)?
    } else {
        EMPTY_RELATIONSHIPS.to_vec()
    };
    let used = relationship_id_numbers(&rels_xml)?;
    let rid = format!("rId{}", lowest_unused_number(&used));
    let patched_rels = insert_before_close(
        &rels_xml,
        "Relationships",
        &format!(r#"<Relationship Id="{rid}" Type="{IMAGE_REL_TYPE}" Target="{rel_target}"/>"#),
    )?;

    let document = entry_bytes(package, "word/document.xml")?;
    let patched_document = patch_paragraph_inline_picture(&document, index, &rid, image_n)?;

    let mut replacements = BTreeMap::from([
        ("word/document.xml".to_owned(), patched_document),
        (DOCUMENT_RELS_NAME.to_owned(), patched_rels),
        (media_name, image_bytes.to_vec()),
    ]);
    ensure_image_content_type(package, extension, content_type, &mut replacements)?;

    let bytes = rebuild_package(package, &replacements)?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn next_media_image_number(package: &[u8]) -> Result<u32> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(format!("invalid DOCX package: {error}")))?;
    let mut max = 0u32;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format_error(format!("cannot list ZIP entry: {error}")))?;
        let name = entry.name();
        let Some(rest) = name.strip_prefix("word/media/image") else {
            continue;
        };
        let Some((digits, _)) = rest.split_once('.') else {
            continue;
        };
        if let Ok(number) = digits.parse::<u32>() {
            max = max.max(number);
        }
    }
    Ok(max + 1)
}

fn ensure_image_content_type(
    package: &[u8],
    extension: &str,
    content_type: &str,
    replacements: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let content_types = if let Some(existing) = replacements.get(CONTENT_TYPES_NAME) {
        existing.clone()
    } else {
        entry_bytes(package, CONTENT_TYPES_NAME)?
    };
    let text = std::str::from_utf8(&content_types)
        .map_err(|error| format_error(format!("content types XML is not UTF-8: {error}")))?;
    let needle = format!(r#"Extension="{extension}""#);
    if text.contains(&needle) {
        return Ok(());
    }
    let insertion = match extension {
        "png" => PNG_DEFAULT,
        "jpeg" => JPEG_DEFAULT,
        "jpg" => JPG_DEFAULT,
        _ => {
            return Err(format_error(format!(
                "unsupported image extension `{extension}` for {content_type}"
            )));
        }
    };
    replacements.insert(
        CONTENT_TYPES_NAME.to_owned(),
        insert_before_close(&content_types, "Types", insertion)?,
    );
    Ok(())
}

fn patch_paragraph_inline_picture(
    xml: &[u8],
    index: u32,
    rid: &str,
    image_n: u32,
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
    let close = paragraph
        .rfind("</w:p>")
        .ok_or_else(|| format_error("unterminated w:p"))?;
    let drawing = inline_picture_run(rid, image_n);
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&paragraph[..close]);
    output.push_str(&drawing);
    output.push_str(&paragraph[close..]);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

fn inline_picture_run(rid: &str, image_n: u32) -> String {
    format!(
        concat!(
            r#"<w:r><w:drawing>"#,
            r#"<wp:inline distT="0" distB="0" distL="0" distR="0" "#,
            r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
            r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
            r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" "#,
            r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
            r#"<wp:extent cx="914400" cy="914400"/>"#,
            r#"<wp:docPr id="{id}" name="Picture {id}"/>"#,
            r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">"#,
            r#"<pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="Picture {id}"/><pic:cNvPicPr/>"#,
            r#"</pic:nvPicPr><pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch>"#,
            r#"</pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/>"#,
            r#"</a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>"#,
            r#"</a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
        ),
        id = image_n,
        rid = rid
    )
}

fn append_comment_entry(xml: &[u8], text: &str, author: &str) -> Result<(u32, Vec<u8>)> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("comments XML is not UTF-8: {error}")))?;
    let used = named_attribute_numbers(source, "w:comment", "w:id")?;
    let comment_id = lowest_unused_number(&used);
    let date = "2024-01-15T12:00:00Z";
    let insertion = format!(
        r#"<w:comment w:id="{comment_id}" w:author="{}" w:date="{date}"><w:p><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p></w:comment>"#,
        xml_escape_attr(author),
        xml_escape(text)
    );
    Ok((
        comment_id,
        insert_before_close(xml, "w:comments", &insertion)?,
    ))
}

fn patch_paragraph_comment_markers(xml: &[u8], index: u32, comment_id: u32) -> Result<Vec<u8>> {
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
    let close = paragraph
        .rfind("</w:p>")
        .ok_or_else(|| format_error("unterminated w:p"))?;
    let markers = format!(r#"<w:commentRangeStart w:id="{comment_id}"/>"#);
    let trailer = format!(
        r#"<w:commentRangeEnd w:id="{comment_id}"/><w:r><w:commentReference w:id="{comment_id}"/></w:r>"#
    );
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    // Insert range start immediately after the opening <w:p ...> tag.
    let gt = paragraph
        .find('>')
        .ok_or_else(|| format_error("unterminated w:p open tag"))?;
    output.push_str(&paragraph[..=gt]);
    output.push_str(&markers);
    output.push_str(&paragraph[gt + 1..close]);
    output.push_str(&trailer);
    output.push_str(&paragraph[close..]);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

fn ensure_comments_package_links(
    package: &[u8],
    replacements: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let rels_exist = has_entry(package, DOCUMENT_RELS_NAME)?;
    let rels_xml = if rels_exist {
        entry_bytes(package, DOCUMENT_RELS_NAME)?
    } else {
        EMPTY_RELATIONSHIPS.to_vec()
    };
    if !relationship_has_type(&rels_xml, COMMENTS_REL_TYPE)? {
        let used = relationship_id_numbers(&rels_xml)?;
        let rid = format!("rId{}", lowest_unused_number(&used));
        replacements.insert(
            DOCUMENT_RELS_NAME.to_owned(),
            insert_before_close(
                &rels_xml,
                "Relationships",
                &format!(
                    r#"<Relationship Id="{rid}" Type="{COMMENTS_REL_TYPE}" Target="comments.xml"/>"#
                ),
            )?,
        );
    }
    let content_types = entry_bytes(package, CONTENT_TYPES_NAME)?;
    if !content_types_has_part(&content_types, "/word/comments.xml")? {
        replacements.insert(
            CONTENT_TYPES_NAME.to_owned(),
            insert_before_close(&content_types, "Types", COMMENTS_OVERRIDE)?,
        );
    }
    Ok(())
}

fn ensure_bullet_numbering(package: &[u8]) -> Result<(u32, Option<Vec<u8>>)> {
    if has_entry(package, NUMBERING_PART)? {
        let xml = entry_bytes(package, NUMBERING_PART)?;
        if let Some(num_id) = find_bullet_num_id(&xml)? {
            return Ok((num_id, None));
        }
        let (num_id, patched) = append_bullet_definition(&xml)?;
        Ok((num_id, Some(patched)))
    } else {
        Ok((1, Some(MINIMAL_BULLET_NUMBERING.to_vec())))
    }
}

fn ensure_numbering_package_links(
    package: &[u8],
    replacements: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let rels_exist = has_entry(package, DOCUMENT_RELS_NAME)?;
    let rels_xml = if rels_exist {
        entry_bytes(package, DOCUMENT_RELS_NAME)?
    } else {
        EMPTY_RELATIONSHIPS.to_vec()
    };
    if !relationship_has_type(&rels_xml, NUMBERING_REL_TYPE)? {
        let used = relationship_id_numbers(&rels_xml)?;
        let rid = format!("rId{}", lowest_unused_number(&used));
        replacements.insert(
            DOCUMENT_RELS_NAME.to_owned(),
            insert_before_close(
                &rels_xml,
                "Relationships",
                &format!(
                    r#"<Relationship Id="{rid}" Type="{NUMBERING_REL_TYPE}" Target="numbering.xml"/>"#
                ),
            )?,
        );
    }
    let content_types = entry_bytes(package, CONTENT_TYPES_NAME)?;
    if !content_types_has_part(&content_types, "/word/numbering.xml")? {
        replacements.insert(
            CONTENT_TYPES_NAME.to_owned(),
            insert_before_close(&content_types, "Types", NUMBERING_OVERRIDE)?,
        );
    }
    Ok(())
}

fn relationship_has_type(xml: &[u8], rel_type: &str) -> Result<bool> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("relationships XML is not UTF-8: {error}")))?;
    Ok(text.contains(rel_type))
}

fn content_types_has_part(xml: &[u8], part_name: &str) -> Result<bool> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("content types XML is not UTF-8: {error}")))?;
    Ok(text.contains(&format!(r#"PartName="{part_name}""#)))
}

fn find_bullet_num_id(xml: &[u8]) -> Result<Option<u32>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("numbering XML is not UTF-8: {error}")))?;
    let mut bullet_abstracts: BTreeSet<u32> = BTreeSet::new();
    let mut cursor = 0;
    while let Some(rel) = find_named_open(&text[cursor..], "w:abstractNum") {
        let start = cursor + rel;
        let end = element_end(text, start, "w:abstractNum")?;
        let block = &text[start..end];
        if block.contains(r#"w:numFmt w:val="bullet""#) {
            let gt = block
                .find('>')
                .ok_or_else(|| format_error("unterminated w:abstractNum"))?;
            if let Some(id) = tag_attribute(&block[..=gt], "w:abstractNumId")
                && let Ok(number) = id.parse()
            {
                bullet_abstracts.insert(number);
            }
        }
        cursor = end;
    }
    if bullet_abstracts.is_empty() {
        return Ok(None);
    }
    cursor = 0;
    while let Some(rel) = find_named_open(&text[cursor..], "w:num") {
        let start = cursor + rel;
        let end = element_end(text, start, "w:num")?;
        let block = &text[start..end];
        let gt = block
            .find('>')
            .ok_or_else(|| format_error("unterminated w:num"))?;
        let num_id = tag_attribute(&block[..=gt], "w:numId")
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| format_error("w:num is missing w:numId"))?;
        if let Some(abstract_id) = extract_named_element(block, "w:abstractNumId")
            && let Some(value) = tag_attribute(abstract_id, "w:val")
            && let Ok(number) = value.parse()
            && bullet_abstracts.contains(&number)
        {
            return Ok(Some(num_id));
        }
        cursor = end;
    }
    Ok(None)
}

fn append_bullet_definition(xml: &[u8]) -> Result<(u32, Vec<u8>)> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("numbering XML is not UTF-8: {error}")))?;
    let abstract_id = lowest_unused_number(&named_attribute_numbers(
        text,
        "w:abstractNum",
        "w:abstractNumId",
    )?);
    let num_id = lowest_unused_number(&named_attribute_numbers(text, "w:num", "w:numId")?);
    let insertion = format!(
        r#"<w:abstractNum w:abstractNumId="{abstract_id}"><w:multiLevelType w:val="hybridMultilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="&#8226;"/><w:lvlJc w:val="left"/></w:lvl></w:abstractNum><w:num w:numId="{num_id}"><w:abstractNumId w:val="{abstract_id}"/></w:num>"#
    );
    Ok((num_id, insert_before_close(xml, "w:numbering", &insertion)?))
}

fn named_attribute_numbers(xml: &str, tag: &str, attribute: &str) -> Result<BTreeSet<u32>> {
    let mut numbers = BTreeSet::new();
    let mut cursor = 0;
    while let Some(rel) = find_named_open(&xml[cursor..], tag) {
        let start = cursor + rel;
        let end = element_end(xml, start, tag)?;
        let gt = xml[start..end]
            .find('>')
            .map(|offset| start + offset)
            .ok_or_else(|| format_error(format!("unterminated `{tag}`")))?;
        if let Some(id) = tag_attribute(&xml[start..=gt], attribute)
            && let Ok(number) = id.parse()
        {
            numbers.insert(number);
        }
        cursor = end;
    }
    Ok(numbers)
}

fn patch_paragraph_num_pr(xml: &[u8], index: u32, num_id: Option<u32>) -> Result<Vec<u8>> {
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
    let replacement = match (num_id, extract_p_pr(&paragraph[open_end..])) {
        (Some(num_id), Some(p_pr)) => {
            let num_pr = num_pr_tag(num_id);
            let patched_p_pr = upsert_num_pr(p_pr, &num_pr)?;
            format!(
                "{}{}{}",
                &paragraph[..open_end],
                patched_p_pr,
                &paragraph[open_end + p_pr.len()..]
            )
        }
        (Some(num_id), None) => {
            format!(
                "{}<w:pPr>{}</w:pPr>{}",
                &paragraph[..open_end],
                num_pr_tag(num_id),
                &paragraph[open_end..]
            )
        }
        (None, Some(p_pr)) => {
            let patched_p_pr = remove_num_pr(p_pr)?;
            format!(
                "{}{}{}",
                &paragraph[..open_end],
                patched_p_pr,
                &paragraph[open_end + p_pr.len()..]
            )
        }
        (None, None) => paragraph.to_owned(),
    };
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&replacement);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

fn num_pr_tag(num_id: u32) -> String {
    format!(r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="{num_id}"/></w:numPr>"#)
}

fn upsert_num_pr(p_pr: &str, num_pr_tag: &str) -> Result<String> {
    if let Some(start) = find_named_open(p_pr, "w:numPr") {
        let end = element_end(p_pr, start, "w:numPr")?;
        return Ok(format!("{}{}{}", &p_pr[..start], num_pr_tag, &p_pr[end..]));
    }
    let open_end = p_pr
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:pPr"))?;
    if p_pr[..open_end].ends_with("/>") {
        let open = p_pr[..open_end].trim_end_matches("/>");
        return Ok(format!("{open}>{num_pr_tag}</w:pPr>"));
    }
    Ok(format!(
        "{}{}{}",
        &p_pr[..open_end],
        num_pr_tag,
        &p_pr[open_end..]
    ))
}

fn remove_num_pr(p_pr: &str) -> Result<String> {
    let Some(start) = find_named_open(p_pr, "w:numPr") else {
        return Ok(p_pr.to_owned());
    };
    let end = element_end(p_pr, start, "w:numPr")?;
    Ok(format!("{}{}", &p_pr[..start], &p_pr[end..]))
}

fn patch_paragraph_hyperlink(xml: &[u8], index: u32, rid: Option<&str>) -> Result<Vec<u8>> {
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
    let patched_paragraph = match rid {
        Some(rid) => wrap_or_update_hyperlink(paragraph, rid)?,
        None => unwrap_hyperlinks(paragraph)?,
    };
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&patched_paragraph);
    output.push_str(&source[span.1..]);
    let with_ns = match rid {
        Some(_) => ensure_r_namespace(&output),
        None => output,
    };
    Ok(with_ns.into_bytes())
}

fn wrap_or_update_hyperlink(paragraph: &str, rid: &str) -> Result<String> {
    if find_named_open(paragraph, "w:hyperlink").is_some() {
        return update_hyperlink_rid(paragraph, rid);
    }
    let (content_start, content_end) = paragraph_content_span(paragraph)?;
    Ok(format!(
        r#"{}<w:hyperlink r:id="{rid}">{}</w:hyperlink>{}"#,
        &paragraph[..content_start],
        &paragraph[content_start..content_end],
        &paragraph[content_end..]
    ))
}

fn update_hyperlink_rid(paragraph: &str, rid: &str) -> Result<String> {
    let start = find_named_open(paragraph, "w:hyperlink")
        .ok_or_else(|| format_error("paragraph is missing w:hyperlink"))?;
    let open_end = paragraph[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| format_error("unterminated w:hyperlink"))?;
    let patched_open = upsert_rid_attr(&paragraph[start..open_end], rid);
    Ok(format!(
        "{}{}{}",
        &paragraph[..start],
        patched_open,
        &paragraph[open_end..]
    ))
}

fn unwrap_hyperlinks(paragraph: &str) -> Result<String> {
    let mut current = paragraph.to_owned();
    while let Some(start) = find_named_open(&current, "w:hyperlink") {
        let end = element_end(&current, start, "w:hyperlink")?;
        let inner = hyperlink_inner(&current[start..end])?;
        current = format!("{}{}{}", &current[..start], inner, &current[end..]);
    }
    Ok(current)
}

fn hyperlink_inner(element: &str) -> Result<&str> {
    let open_end = element
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:hyperlink"))?;
    if element[..open_end].ends_with("/>") {
        return Ok("");
    }
    let close = "</w:hyperlink>";
    let inner_end = element
        .len()
        .checked_sub(close.len())
        .filter(|end| *end >= open_end)
        .ok_or_else(|| format_error("unterminated w:hyperlink"))?;
    Ok(&element[open_end..inner_end])
}

fn paragraph_content_span(paragraph: &str) -> Result<(usize, usize)> {
    let open_end = paragraph
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:p"))?;
    if paragraph[..open_end].ends_with("/>") {
        return Err(format_error("cannot hyperlink a self-closing paragraph"));
    }
    let content_start = match extract_p_pr(&paragraph[open_end..]) {
        Some(p_pr) => open_end + p_pr.len(),
        None => open_end,
    };
    let content_end = paragraph
        .rfind("</w:p>")
        .ok_or_else(|| format_error("unterminated w:p"))?;
    Ok((content_start, content_end))
}

fn upsert_rid_attr(open_tag: &str, rid: &str) -> String {
    let needle = r#"r:id=""#;
    if let Some(at) = open_tag.find(needle) {
        let value_start = at + needle.len();
        if let Some(rel) = open_tag[value_start..].find('"') {
            let value_end = value_start + rel;
            return format!(
                "{}{rid}{}",
                &open_tag[..value_start],
                &open_tag[value_end..]
            );
        }
    }
    if let Some(stripped) = open_tag.strip_suffix("/>") {
        format!(r#"{stripped} r:id="{rid}"/>"#)
    } else if let Some(stripped) = open_tag.strip_suffix('>') {
        format!(r#"{stripped} r:id="{rid}">"#)
    } else {
        format!(r#"{open_tag} r:id="{rid}""#)
    }
}

fn ensure_r_namespace(xml: &str) -> String {
    if xml.contains("xmlns:r=") {
        return xml.to_owned();
    }
    let Some(start) = xml.find("<w:document") else {
        return xml.to_owned();
    };
    let Some(rel) = xml[start..].find('>') else {
        return xml.to_owned();
    };
    let gt = start + rel;
    if xml[..gt].ends_with('/') {
        format!("{} {R_NAMESPACE}/>{}", &xml[..gt - 1], &xml[gt + 1..])
    } else {
        format!("{} {R_NAMESPACE}>{}", &xml[..gt], &xml[gt + 1..])
    }
}

fn existing_paragraph_hyperlink_rid(xml: &[u8], index: u32) -> Result<Option<String>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("document XML is not UTF-8: {error}")))?;
    let spans = paragraph_spans(source)?;
    let span = spans
        .get(index as usize)
        .ok_or_else(|| format_error(format!("paragraph `{index}` was not found")))?;
    let paragraph = &source[span.0..span.1];
    let Some(start) = find_named_open(paragraph, "w:hyperlink") else {
        return Ok(None);
    };
    let open_end = paragraph[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| format_error("unterminated w:hyperlink"))?;
    Ok(tag_attribute(&paragraph[start..open_end], "r:id"))
}

fn upsert_hyperlink_relationship(xml: &[u8], rid: &str, url: &str) -> Result<Vec<u8>> {
    let without = remove_relationship(xml, rid)?;
    insert_before_close(
        &without,
        "Relationships",
        &hyperlink_relationship_tag(rid, url),
    )
}

fn hyperlink_relationship_tag(rid: &str, url: &str) -> String {
    format!(
        r#"<Relationship Id="{rid}" Type="{HYPERLINK_REL_TYPE}" Target="{}" TargetMode="External"/>"#,
        xml_escape_attr(url)
    )
}

fn insert_before_close(xml: &[u8], element: &str, insertion: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("XML is not UTF-8: {error}")))?;
    let closing = format!("</{element}>");
    let position = text
        .rfind(&closing)
        .ok_or_else(|| format_error(format!("XML is missing closing `{element}`")))?;
    Ok(format!("{}{}{}", &text[..position], insertion, &text[position..]).into_bytes())
}

fn remove_relationship(xml: &[u8], id: &str) -> Result<Vec<u8>> {
    remove_matching_tag(xml, "Relationship", |tag| {
        tag_attribute(tag, "Id").as_deref() == Some(id)
    })
}

fn remove_matching_tag(
    xml: &[u8],
    tag_name: &str,
    matches: impl Fn(&str) -> bool,
) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("XML is not UTF-8: {error}")))?;
    let needle = format!("<{tag_name}");
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find(&needle) {
        let start = cursor + offset;
        let after_name = &text[start + needle.len()..];
        if after_name.starts_with(|character: char| character.is_ascii_alphabetic()) {
            output.push_str(&text[cursor..start + needle.len()]);
            cursor = start + needle.len();
            continue;
        }
        let end = text[start..]
            .find('>')
            .map(|rel| start + rel + 1)
            .ok_or_else(|| format_error(format!("unterminated `{tag_name}`")))?;
        let tag = &text[start..end];
        let self_closing = tag.ends_with("/>");
        let span_end = if self_closing {
            end
        } else {
            let close = format!("</{tag_name}>");
            text[end..]
                .find(&close)
                .map(|rel| end + rel + close.len())
                .ok_or_else(|| format_error(format!("missing close for `{tag_name}`")))?
        };
        if matches(tag) {
            output.push_str(&text[cursor..start]);
            cursor = span_end;
        } else {
            output.push_str(&text[cursor..span_end]);
            cursor = span_end;
        }
    }
    output.push_str(&text[cursor..]);
    Ok(output.into_bytes())
}

fn relationship_id_numbers(xml: &[u8]) -> Result<BTreeSet<u32>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("relationships XML is not UTF-8: {error}")))?;
    let mut numbers = BTreeSet::new();
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find("<Relationship") {
        let start = cursor + rel;
        let end = text[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| format_error("unterminated Relationship"))?;
        if let Some(id) = tag_attribute(&text[start..end], "Id")
            && let Some(number) = id.strip_prefix("rId").and_then(|value| value.parse().ok())
        {
            numbers.insert(number);
        }
        cursor = end;
    }
    Ok(numbers)
}

fn lowest_unused_number(used: &BTreeSet<u32>) -> u32 {
    let mut number = 1;
    while used.contains(&number) {
        number += 1;
    }
    number
}

fn tag_attribute(tag: &str, attribute: &str) -> Option<String> {
    let needle = format!("{attribute}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

fn has_entry(package: &[u8], name: &str) -> Result<bool> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(format!("invalid DOCX package: {error}")))?;
    Ok(archive.by_name(name).is_ok())
}

pub fn patch_paragraph_font_size(xml: &[u8], index: u32, size_pt: Option<f64>) -> Result<Vec<u8>> {
    match size_pt {
        Some(pt) => {
            let half = (pt * 2.0).round() as i64;
            if half <= 0 {
                return Err(format_error("`size_pt` must be a positive number"));
            }
            let prop = format!(r#"<w:sz w:val="{half}"/><w:szCs w:val="{half}"/>"#);
            patch_paragraph_font_size_props(xml, index, Some(&prop))
        }
        None => patch_paragraph_font_size_props(xml, index, None),
    }
}

fn patch_paragraph_font_size_props(
    xml: &[u8],
    index: u32,
    prop_tag: Option<&str>,
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
    let patched_paragraph = match prop_tag {
        Some(prop) => set_runs_font_size(paragraph, prop)?,
        None => clear_runs_font_size(paragraph)?,
    };
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&patched_paragraph);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

fn set_runs_font_size(paragraph: &str, prop_tag: &str) -> Result<String> {
    // Upsert both w:sz and w:szCs by replacing any existing pair with prop_tag.
    let mut output = String::with_capacity(paragraph.len() + 32);
    let mut cursor = 0;
    let mut patched_any = false;
    while let Some(rel) = paragraph[cursor..].find("<w:r") {
        let start = cursor + rel;
        let after = paragraph.as_bytes().get(start + 4).copied().unwrap_or(0);
        if after != b' ' && after != b'>' && after != b'/' {
            output.push_str(&paragraph[cursor..start + 4]);
            cursor = start + 4;
            continue;
        }
        let end = element_end(paragraph, start, "w:r")?;
        output.push_str(&paragraph[cursor..start]);
        output.push_str(&upsert_run_font_size(&paragraph[start..end], prop_tag)?);
        cursor = end;
        patched_any = true;
    }
    output.push_str(&paragraph[cursor..]);
    if !patched_any {
        return upsert_paragraph_mark_font_size(paragraph, Some(prop_tag));
    }
    Ok(output)
}

fn clear_runs_font_size(paragraph: &str) -> Result<String> {
    let mut output = String::with_capacity(paragraph.len());
    let mut cursor = 0;
    let mut patched_any = false;
    while let Some(rel) = paragraph[cursor..].find("<w:r") {
        let start = cursor + rel;
        let after = paragraph.as_bytes().get(start + 4).copied().unwrap_or(0);
        if after != b' ' && after != b'>' && after != b'/' {
            output.push_str(&paragraph[cursor..start + 4]);
            cursor = start + 4;
            continue;
        }
        let end = element_end(paragraph, start, "w:r")?;
        output.push_str(&paragraph[cursor..start]);
        output.push_str(&clear_run_font_size(&paragraph[start..end])?);
        cursor = end;
        patched_any = true;
    }
    output.push_str(&paragraph[cursor..]);
    if !patched_any {
        return upsert_paragraph_mark_font_size(paragraph, None);
    }
    Ok(output)
}

fn upsert_run_font_size(run: &str, prop_tag: &str) -> Result<String> {
    let open_end = run
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:r"))?;
    if run[..open_end].ends_with("/>") {
        return Ok(run.to_owned());
    }
    let rest = &run[open_end..];
    if let Some(r_pr) = extract_named_element(rest, "w:rPr") {
        let patched_r_pr = upsert_font_size_in_r_pr(r_pr, Some(prop_tag))?;
        return Ok(format!(
            "{}{}{}",
            &run[..open_end],
            patched_r_pr,
            &rest[r_pr.len()..]
        ));
    }
    Ok(format!(
        "{}<w:rPr>{}</w:rPr>{}",
        &run[..open_end],
        prop_tag,
        rest
    ))
}

fn clear_run_font_size(run: &str) -> Result<String> {
    let open_end = run
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:r"))?;
    if run[..open_end].ends_with("/>") {
        return Ok(run.to_owned());
    }
    let rest = &run[open_end..];
    let Some(r_pr) = extract_named_element(rest, "w:rPr") else {
        return Ok(run.to_owned());
    };
    let patched_r_pr = upsert_font_size_in_r_pr(r_pr, None)?;
    Ok(format!(
        "{}{}{}",
        &run[..open_end],
        patched_r_pr,
        &rest[r_pr.len()..]
    ))
}

fn upsert_paragraph_mark_font_size(paragraph: &str, prop_tag: Option<&str>) -> Result<String> {
    let open_end = paragraph
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:p"))?;
    match extract_p_pr(&paragraph[open_end..]) {
        Some(p_pr) => {
            let patched = if let Some(start) = find_named_open(p_pr, "w:rPr") {
                let end = element_end(p_pr, start, "w:rPr")?;
                format!(
                    "{}{}{}",
                    &p_pr[..start],
                    upsert_font_size_in_r_pr(&p_pr[start..end], prop_tag)?,
                    &p_pr[end..]
                )
            } else if let Some(prop) = prop_tag {
                let p_open_end = p_pr
                    .find('>')
                    .map(|offset| offset + 1)
                    .ok_or_else(|| format_error("unterminated w:pPr"))?;
                if p_pr[..p_open_end].ends_with("/>") {
                    let open = p_pr[..p_open_end].trim_end_matches("/>");
                    format!("{open}><w:rPr>{prop}</w:rPr></w:pPr>")
                } else {
                    format!(
                        "{}<w:rPr>{prop}</w:rPr>{}",
                        &p_pr[..p_open_end],
                        &p_pr[p_open_end..]
                    )
                }
            } else {
                p_pr.to_owned()
            };
            Ok(format!(
                "{}{}{}",
                &paragraph[..open_end],
                patched,
                &paragraph[open_end + p_pr.len()..]
            ))
        }
        None => {
            if let Some(prop) = prop_tag {
                Ok(format!(
                    "{}<w:pPr><w:rPr>{prop}</w:rPr></w:pPr>{}",
                    &paragraph[..open_end],
                    &paragraph[open_end..]
                ))
            } else {
                Ok(paragraph.to_owned())
            }
        }
    }
}

fn upsert_font_size_in_r_pr(r_pr: &str, prop_tag: Option<&str>) -> Result<String> {
    let without_sz = remove_named_child(r_pr, "w:sz")?;
    let without_both = remove_named_child(&without_sz, "w:szCs")?;
    let Some(prop) = prop_tag else {
        return Ok(without_both);
    };
    let open_end = without_both
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:rPr"))?;
    if without_both[..open_end].ends_with("/>") {
        let open = without_both[..open_end].trim_end_matches("/>");
        return Ok(format!("{open}>{prop}</w:rPr>"));
    }
    Ok(format!(
        "{}{}{}",
        &without_both[..open_end],
        prop,
        &without_both[open_end..]
    ))
}

pub fn patch_paragraph_font_name(xml: &[u8], index: u32, font: Option<&str>) -> Result<Vec<u8>> {
    match font {
        Some(name) => {
            let escaped = escape_xml_attr(name);
            let prop =
                format!(r#"<w:rFonts w:ascii="{escaped}" w:hAnsi="{escaped}" w:cs="{escaped}"/>"#);
            patch_paragraph_font_name_props(xml, index, Some(&prop))
        }
        None => patch_paragraph_font_name_props(xml, index, None),
    }
}

fn patch_paragraph_font_name_props(
    xml: &[u8],
    index: u32,
    prop_tag: Option<&str>,
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
    let patched_paragraph = match prop_tag {
        Some(prop) => set_runs_font_name(paragraph, prop)?,
        None => clear_runs_font_name(paragraph)?,
    };
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&patched_paragraph);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

fn set_runs_font_name(paragraph: &str, prop_tag: &str) -> Result<String> {
    let mut output = String::with_capacity(paragraph.len() + 32);
    let mut cursor = 0;
    let mut patched_any = false;
    while let Some(rel) = paragraph[cursor..].find("<w:r") {
        let start = cursor + rel;
        let after = paragraph.as_bytes().get(start + 4).copied().unwrap_or(0);
        if after != b' ' && after != b'>' && after != b'/' {
            output.push_str(&paragraph[cursor..start + 4]);
            cursor = start + 4;
            continue;
        }
        let end = element_end(paragraph, start, "w:r")?;
        output.push_str(&paragraph[cursor..start]);
        output.push_str(&upsert_run_font_name(&paragraph[start..end], prop_tag)?);
        cursor = end;
        patched_any = true;
    }
    output.push_str(&paragraph[cursor..]);
    if !patched_any {
        return upsert_paragraph_mark_font_name(paragraph, Some(prop_tag));
    }
    Ok(output)
}

fn clear_runs_font_name(paragraph: &str) -> Result<String> {
    let mut output = String::with_capacity(paragraph.len());
    let mut cursor = 0;
    let mut patched_any = false;
    while let Some(rel) = paragraph[cursor..].find("<w:r") {
        let start = cursor + rel;
        let after = paragraph.as_bytes().get(start + 4).copied().unwrap_or(0);
        if after != b' ' && after != b'>' && after != b'/' {
            output.push_str(&paragraph[cursor..start + 4]);
            cursor = start + 4;
            continue;
        }
        let end = element_end(paragraph, start, "w:r")?;
        output.push_str(&paragraph[cursor..start]);
        output.push_str(&clear_run_font_name(&paragraph[start..end])?);
        cursor = end;
        patched_any = true;
    }
    output.push_str(&paragraph[cursor..]);
    if !patched_any {
        return upsert_paragraph_mark_font_name(paragraph, None);
    }
    Ok(output)
}

fn upsert_run_font_name(run: &str, prop_tag: &str) -> Result<String> {
    let open_end = run
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:r"))?;
    if run[..open_end].ends_with("/>") {
        return Ok(run.to_owned());
    }
    let rest = &run[open_end..];
    if let Some(r_pr) = extract_named_element(rest, "w:rPr") {
        let patched_r_pr = upsert_font_name_in_r_pr(r_pr, Some(prop_tag))?;
        return Ok(format!(
            "{}{}{}",
            &run[..open_end],
            patched_r_pr,
            &rest[r_pr.len()..]
        ));
    }
    Ok(format!(
        "{}<w:rPr>{}</w:rPr>{}",
        &run[..open_end],
        prop_tag,
        rest
    ))
}

fn clear_run_font_name(run: &str) -> Result<String> {
    let open_end = run
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:r"))?;
    if run[..open_end].ends_with("/>") {
        return Ok(run.to_owned());
    }
    let rest = &run[open_end..];
    let Some(r_pr) = extract_named_element(rest, "w:rPr") else {
        return Ok(run.to_owned());
    };
    let patched_r_pr = upsert_font_name_in_r_pr(r_pr, None)?;
    Ok(format!(
        "{}{}{}",
        &run[..open_end],
        patched_r_pr,
        &rest[r_pr.len()..]
    ))
}

fn upsert_paragraph_mark_font_name(paragraph: &str, prop_tag: Option<&str>) -> Result<String> {
    let open_end = paragraph
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:p"))?;
    match extract_p_pr(&paragraph[open_end..]) {
        Some(p_pr) => {
            let patched = if let Some(start) = find_named_open(p_pr, "w:rPr") {
                let end = element_end(p_pr, start, "w:rPr")?;
                format!(
                    "{}{}{}",
                    &p_pr[..start],
                    upsert_font_name_in_r_pr(&p_pr[start..end], prop_tag)?,
                    &p_pr[end..]
                )
            } else if let Some(prop) = prop_tag {
                let p_open_end = p_pr
                    .find('>')
                    .map(|offset| offset + 1)
                    .ok_or_else(|| format_error("unterminated w:pPr"))?;
                if p_pr[..p_open_end].ends_with("/>") {
                    let open = p_pr[..p_open_end].trim_end_matches("/>");
                    format!("{open}><w:rPr>{prop}</w:rPr></w:pPr>")
                } else {
                    format!(
                        "{}<w:rPr>{prop}</w:rPr>{}",
                        &p_pr[..p_open_end],
                        &p_pr[p_open_end..]
                    )
                }
            } else {
                p_pr.to_owned()
            };
            Ok(format!(
                "{}{}{}",
                &paragraph[..open_end],
                patched,
                &paragraph[open_end + p_pr.len()..]
            ))
        }
        None => {
            if let Some(prop) = prop_tag {
                Ok(format!(
                    "{}<w:pPr><w:rPr>{prop}</w:rPr></w:pPr>{}",
                    &paragraph[..open_end],
                    &paragraph[open_end..]
                ))
            } else {
                Ok(paragraph.to_owned())
            }
        }
    }
}

fn upsert_font_name_in_r_pr(r_pr: &str, prop_tag: Option<&str>) -> Result<String> {
    let without = remove_named_child(r_pr, "w:rFonts")?;
    let Some(prop) = prop_tag else {
        return Ok(without);
    };
    let open_end = without
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:rPr"))?;
    if without[..open_end].ends_with("/>") {
        let open = without[..open_end].trim_end_matches("/>");
        return Ok(format!("{open}>{prop}</w:rPr>"));
    }
    Ok(format!(
        "{}{}{}",
        &without[..open_end],
        prop,
        &without[open_end..]
    ))
}

pub fn patch_paragraph_font_color(xml: &[u8], index: u32, color: Option<&str>) -> Result<Vec<u8>> {
    match color {
        Some(hex) => {
            let prop = format!(r#"<w:color w:val="{hex}"/>"#);
            patch_paragraph_named_run_prop(xml, index, "w:color", Some(&prop))
        }
        None => patch_paragraph_named_run_prop(xml, index, "w:color", None),
    }
}

pub fn patch_cell_shading(xml: &[u8], index: u32, color: Option<&str>) -> Result<Vec<u8>> {
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
    let Some((cell_start, cell_end)) = enclosing_table_cell(source, span.0, span.1)? else {
        return Err(format_error("paragraph is not inside a table cell"));
    };
    let cell = &source[cell_start..cell_end];
    let patched_cell = upsert_cell_shading(cell, color)?;
    let mut output = String::new();
    output.push_str(&source[..cell_start]);
    output.push_str(&patched_cell);
    output.push_str(&source[cell_end..]);
    Ok(output.into_bytes())
}

fn enclosing_table_cell(
    xml: &str,
    para_start: usize,
    para_end: usize,
) -> Result<Option<(usize, usize)>> {
    let prefix = &xml[..para_start];
    let mut search_end = prefix.len();
    while let Some(rel) = prefix[..search_end].rfind("<w:tc") {
        let after = prefix.as_bytes().get(rel + 5).copied().unwrap_or(0);
        if after == b' ' || after == b'>' || after == b'/' {
            let end = element_end(xml, rel, "w:tc")?;
            if end >= para_end {
                return Ok(Some((rel, end)));
            }
        }
        if rel == 0 {
            break;
        }
        search_end = rel;
    }
    Ok(None)
}

fn upsert_cell_shading(cell: &str, color: Option<&str>) -> Result<String> {
    let open_end = cell
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:tc"))?;
    if cell[..open_end].ends_with("/>") {
        return Err(format_error("empty table cell cannot be shaded"));
    }
    let inner = &cell[open_end..];
    match extract_named_element(inner, "w:tcPr") {
        Some(tc_pr) => {
            let start_in_inner = find_named_open(inner, "w:tcPr")
                .ok_or_else(|| format_error("w:tcPr vanished while patching cell shading"))?;
            let patched_tc_pr = upsert_shd_in_tc_pr(tc_pr, color)?;
            Ok(format!(
                "{}{}{}{}",
                &cell[..open_end],
                &inner[..start_in_inner],
                patched_tc_pr,
                &inner[start_in_inner + tc_pr.len()..]
            ))
        }
        None => {
            let Some(fill) = color else {
                return Ok(cell.to_owned());
            };
            let tc_pr = format!(
                r#"<w:tcPr><w:shd w:val="clear" w:color="auto" w:fill="{fill}"/></w:tcPr>"#
            );
            Ok(format!(
                "{}{}{}",
                &cell[..open_end],
                tc_pr,
                &cell[open_end..]
            ))
        }
    }
}

fn upsert_shd_in_tc_pr(tc_pr: &str, color: Option<&str>) -> Result<String> {
    match color {
        None => remove_named_child(tc_pr, "w:shd"),
        Some(fill) => {
            let shd = format!(r#"<w:shd w:val="clear" w:color="auto" w:fill="{fill}"/>"#);
            if let Some(existing) = extract_named_element(tc_pr, "w:shd") {
                let start = find_named_open(tc_pr, "w:shd")
                    .ok_or_else(|| format_error("w:shd vanished while patching cell shading"))?;
                let patched_shd = set_shd_fill(existing, fill)?;
                Ok(format!(
                    "{}{}{}",
                    &tc_pr[..start],
                    patched_shd,
                    &tc_pr[start + existing.len()..]
                ))
            } else {
                let open_end = tc_pr
                    .find('>')
                    .map(|offset| offset + 1)
                    .ok_or_else(|| format_error("unterminated w:tcPr"))?;
                if tc_pr[..open_end].ends_with("/>") {
                    let open = tc_pr[..open_end].trim_end_matches("/>");
                    return Ok(format!("{open}>{shd}</w:tcPr>"));
                }
                Ok(format!(
                    "{}{}{}",
                    &tc_pr[..open_end],
                    shd,
                    &tc_pr[open_end..]
                ))
            }
        }
    }
}

fn set_shd_fill(shd: &str, fill: &str) -> Result<String> {
    const FILL_ATTR: &str = "w:fill=\"";
    if let Some(rel) = shd.find(FILL_ATTR) {
        let value_start = rel + FILL_ATTR.len();
        let value_end = shd[value_start..]
            .find('"')
            .map(|offset| value_start + offset)
            .ok_or_else(|| format_error("unterminated w:fill"))?;
        return Ok(format!(
            "{}{}{}",
            &shd[..value_start],
            fill,
            &shd[value_end..]
        ));
    }
    let gt = shd
        .find('>')
        .ok_or_else(|| format_error("unterminated w:shd"))?;
    if shd.as_bytes().get(gt.saturating_sub(1)) == Some(&b'/') {
        Ok(format!(
            r#"{} w:fill="{}"{}"#,
            &shd[..gt - 1],
            fill,
            &shd[gt - 1..]
        ))
    } else {
        Ok(format!(r#"{} w:fill="{}"{}"#, &shd[..gt], fill, &shd[gt..]))
    }
}

pub fn patch_paragraph_highlight(xml: &[u8], index: u32, color: Option<&str>) -> Result<Vec<u8>> {
    match color {
        Some(name) => {
            let prop = format!(r#"<w:highlight w:val="{name}"/>"#);
            patch_paragraph_named_run_prop(xml, index, "w:highlight", Some(&prop))
        }
        None => patch_paragraph_named_run_prop(xml, index, "w:highlight", None),
    }
}

fn patch_paragraph_named_run_prop(
    xml: &[u8],
    index: u32,
    tag: &str,
    prop_tag: Option<&str>,
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
    let patched_paragraph = match prop_tag {
        Some(prop) => set_runs_named_prop(paragraph, tag, prop)?,
        None => clear_runs_named_prop(paragraph, tag)?,
    };
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&patched_paragraph);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

fn set_runs_named_prop(paragraph: &str, tag: &str, prop_tag: &str) -> Result<String> {
    let mut output = String::with_capacity(paragraph.len() + 32);
    let mut cursor = 0;
    let mut patched_any = false;
    while let Some(rel) = paragraph[cursor..].find("<w:r") {
        let start = cursor + rel;
        let after = paragraph.as_bytes().get(start + 4).copied().unwrap_or(0);
        if after != b' ' && after != b'>' && after != b'/' {
            output.push_str(&paragraph[cursor..start + 4]);
            cursor = start + 4;
            continue;
        }
        let end = element_end(paragraph, start, "w:r")?;
        output.push_str(&paragraph[cursor..start]);
        output.push_str(&upsert_run_named_prop(
            &paragraph[start..end],
            tag,
            prop_tag,
        )?);
        cursor = end;
        patched_any = true;
    }
    output.push_str(&paragraph[cursor..]);
    if !patched_any {
        return upsert_paragraph_mark_named_prop(paragraph, tag, Some(prop_tag));
    }
    Ok(output)
}

fn clear_runs_named_prop(paragraph: &str, tag: &str) -> Result<String> {
    let mut output = String::with_capacity(paragraph.len());
    let mut cursor = 0;
    let mut patched_any = false;
    while let Some(rel) = paragraph[cursor..].find("<w:r") {
        let start = cursor + rel;
        let after = paragraph.as_bytes().get(start + 4).copied().unwrap_or(0);
        if after != b' ' && after != b'>' && after != b'/' {
            output.push_str(&paragraph[cursor..start + 4]);
            cursor = start + 4;
            continue;
        }
        let end = element_end(paragraph, start, "w:r")?;
        output.push_str(&paragraph[cursor..start]);
        output.push_str(&clear_run_named_prop(&paragraph[start..end], tag)?);
        cursor = end;
        patched_any = true;
    }
    output.push_str(&paragraph[cursor..]);
    if !patched_any {
        return upsert_paragraph_mark_named_prop(paragraph, tag, None);
    }
    Ok(output)
}

fn upsert_run_named_prop(run: &str, tag: &str, prop_tag: &str) -> Result<String> {
    let open_end = run
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:r"))?;
    if run[..open_end].ends_with("/>") {
        return Ok(run.to_owned());
    }
    let rest = &run[open_end..];
    if let Some(r_pr) = extract_named_element(rest, "w:rPr") {
        let patched_r_pr = upsert_named_prop_in_r_pr(r_pr, tag, Some(prop_tag))?;
        return Ok(format!(
            "{}{}{}",
            &run[..open_end],
            patched_r_pr,
            &rest[r_pr.len()..]
        ));
    }
    Ok(format!(
        "{}<w:rPr>{}</w:rPr>{}",
        &run[..open_end],
        prop_tag,
        rest
    ))
}

fn clear_run_named_prop(run: &str, tag: &str) -> Result<String> {
    let open_end = run
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:r"))?;
    if run[..open_end].ends_with("/>") {
        return Ok(run.to_owned());
    }
    let rest = &run[open_end..];
    let Some(r_pr) = extract_named_element(rest, "w:rPr") else {
        return Ok(run.to_owned());
    };
    let patched_r_pr = upsert_named_prop_in_r_pr(r_pr, tag, None)?;
    Ok(format!(
        "{}{}{}",
        &run[..open_end],
        patched_r_pr,
        &rest[r_pr.len()..]
    ))
}

fn upsert_paragraph_mark_named_prop(
    paragraph: &str,
    tag: &str,
    prop_tag: Option<&str>,
) -> Result<String> {
    let open_end = paragraph
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:p"))?;
    match extract_p_pr(&paragraph[open_end..]) {
        Some(p_pr) => {
            let patched = if let Some(start) = find_named_open(p_pr, "w:rPr") {
                let end = element_end(p_pr, start, "w:rPr")?;
                format!(
                    "{}{}{}",
                    &p_pr[..start],
                    upsert_named_prop_in_r_pr(&p_pr[start..end], tag, prop_tag)?,
                    &p_pr[end..]
                )
            } else if let Some(prop) = prop_tag {
                let p_open_end = p_pr
                    .find('>')
                    .map(|offset| offset + 1)
                    .ok_or_else(|| format_error("unterminated w:pPr"))?;
                if p_pr[..p_open_end].ends_with("/>") {
                    let open = p_pr[..p_open_end].trim_end_matches("/>");
                    format!("{open}><w:rPr>{prop}</w:rPr></w:pPr>")
                } else {
                    format!(
                        "{}<w:rPr>{prop}</w:rPr>{}",
                        &p_pr[..p_open_end],
                        &p_pr[p_open_end..]
                    )
                }
            } else {
                p_pr.to_owned()
            };
            Ok(format!(
                "{}{}{}",
                &paragraph[..open_end],
                patched,
                &paragraph[open_end + p_pr.len()..]
            ))
        }
        None => {
            if let Some(prop) = prop_tag {
                Ok(format!(
                    "{}<w:pPr><w:rPr>{prop}</w:rPr></w:pPr>{}",
                    &paragraph[..open_end],
                    &paragraph[open_end..]
                ))
            } else {
                Ok(paragraph.to_owned())
            }
        }
    }
}

fn upsert_named_prop_in_r_pr(r_pr: &str, tag: &str, prop_tag: Option<&str>) -> Result<String> {
    let without = remove_named_child(r_pr, tag)?;
    let Some(prop) = prop_tag else {
        return Ok(without);
    };
    let open_end = without
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:rPr"))?;
    if without[..open_end].ends_with("/>") {
        let open = without[..open_end].trim_end_matches("/>");
        return Ok(format!("{open}>{prop}</w:rPr>"));
    }
    Ok(format!(
        "{}{}{}",
        &without[..open_end],
        prop,
        &without[open_end..]
    ))
}

fn escape_xml_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn remove_named_child(parent: &str, tag: &str) -> Result<String> {
    let Some(start) = find_named_open(parent, tag) else {
        return Ok(parent.to_owned());
    };
    let end = element_end(parent, start, tag)?;
    Ok(format!("{}{}", &parent[..start], &parent[end..]))
}

fn patch_paragraph_run_bool(xml: &[u8], index: u32, tag: &str, enabled: bool) -> Result<Vec<u8>> {
    patch_paragraph_run_prop(xml, index, tag, &toggle_tag(tag, enabled))
}

fn patch_paragraph_run_prop(xml: &[u8], index: u32, tag: &str, prop_tag: &str) -> Result<Vec<u8>> {
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
    let patched_paragraph = set_runs_prop(paragraph, tag, prop_tag)?;
    let mut output = String::new();
    output.push_str(&source[..span.0]);
    output.push_str(&patched_paragraph);
    output.push_str(&source[span.1..]);
    Ok(output.into_bytes())
}

fn set_runs_prop(paragraph: &str, tag: &str, prop_tag: &str) -> Result<String> {
    let mut output = String::with_capacity(paragraph.len() + 32);
    let mut cursor = 0;
    let mut patched_any = false;
    while let Some(rel) = paragraph[cursor..].find("<w:r") {
        let start = cursor + rel;
        let after = paragraph.as_bytes().get(start + 4).copied().unwrap_or(0);
        if after != b' ' && after != b'>' && after != b'/' {
            output.push_str(&paragraph[cursor..start + 4]);
            cursor = start + 4;
            continue;
        }
        let end = element_end(paragraph, start, "w:r")?;
        output.push_str(&paragraph[cursor..start]);
        output.push_str(&upsert_run_prop(&paragraph[start..end], tag, prop_tag)?);
        cursor = end;
        patched_any = true;
    }
    output.push_str(&paragraph[cursor..]);
    if !patched_any {
        return upsert_paragraph_mark_prop(paragraph, tag, prop_tag);
    }
    Ok(output)
}

fn toggle_tag(tag: &str, enabled: bool) -> String {
    if enabled {
        format!("<{tag}/>")
    } else {
        format!(r#"<{tag} w:val="0"/>"#)
    }
}

fn upsert_run_prop(run: &str, tag: &str, prop_tag: &str) -> Result<String> {
    let open_end = run
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:r"))?;
    if run[..open_end].ends_with("/>") {
        return Ok(run.to_owned());
    }
    let rest = &run[open_end..];
    if let Some(r_pr) = extract_named_element(rest, "w:rPr") {
        let patched_r_pr = upsert_toggle_in_r_pr(r_pr, tag, prop_tag)?;
        return Ok(format!(
            "{}{}{}",
            &run[..open_end],
            patched_r_pr,
            &rest[r_pr.len()..]
        ));
    }
    Ok(format!(
        "{}<w:rPr>{}</w:rPr>{}",
        &run[..open_end],
        prop_tag,
        rest
    ))
}

fn upsert_paragraph_mark_prop(paragraph: &str, tag: &str, prop_tag: &str) -> Result<String> {
    let open_end = paragraph
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:p"))?;
    let r_pr_inner = format!("<w:rPr>{prop_tag}</w:rPr>");
    match extract_p_pr(&paragraph[open_end..]) {
        Some(p_pr) => {
            let patched = if let Some(start) = find_named_open(p_pr, "w:rPr") {
                let end = element_end(p_pr, start, "w:rPr")?;
                format!(
                    "{}{}{}",
                    &p_pr[..start],
                    upsert_toggle_in_r_pr(&p_pr[start..end], tag, prop_tag)?,
                    &p_pr[end..]
                )
            } else {
                let p_open_end = p_pr
                    .find('>')
                    .map(|offset| offset + 1)
                    .ok_or_else(|| format_error("unterminated w:pPr"))?;
                if p_pr[..p_open_end].ends_with("/>") {
                    let open = p_pr[..p_open_end].trim_end_matches("/>");
                    format!("{open}>{r_pr_inner}</w:pPr>")
                } else {
                    format!(
                        "{}{}{}",
                        &p_pr[..p_open_end],
                        r_pr_inner,
                        &p_pr[p_open_end..]
                    )
                }
            };
            Ok(format!(
                "{}{}{}",
                &paragraph[..open_end],
                patched,
                &paragraph[open_end + p_pr.len()..]
            ))
        }
        None => Ok(format!(
            "{}<w:pPr>{r_pr_inner}</w:pPr>{}",
            &paragraph[..open_end],
            &paragraph[open_end..]
        )),
    }
}

fn upsert_toggle_in_r_pr(r_pr: &str, tag: &str, prop_tag: &str) -> Result<String> {
    if let Some(start) = find_named_open(r_pr, tag) {
        let end = element_end(r_pr, start, tag)?;
        return Ok(format!("{}{}{}", &r_pr[..start], prop_tag, &r_pr[end..]));
    }
    let open_end = r_pr
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated w:rPr"))?;
    if r_pr[..open_end].ends_with("/>") {
        let open = r_pr[..open_end].trim_end_matches("/>");
        return Ok(format!("{open}>{prop_tag}</w:rPr>"));
    }
    Ok(format!(
        "{}{}{}",
        &r_pr[..open_end],
        prop_tag,
        &r_pr[open_end..]
    ))
}

fn extract_named_element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let start = find_named_open(xml, tag)?;
    let end = element_end(xml, start, tag).ok()?;
    Some(&xml[start..end])
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
        .or_else(|| payload.get("paragraph"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format_error("`index` or `element_id` is required"))? as u32;
    model
        .paragraphs
        .iter()
        .find(|paragraph| paragraph.index == index)
        .ok_or_else(|| format_error(format!("paragraph `{index}` was not found")))
}

fn resolve_hyperlink_paragraph<'a>(
    model: &'a DocumentModel,
    payload: &serde_json::Value,
) -> Result<&'a crate::model::ParagraphModel> {
    if payload
        .get("element_id")
        .and_then(serde_json::Value::as_str)
        .is_some()
        || payload
            .get("index")
            .and_then(serde_json::Value::as_u64)
            .is_some()
    {
        return resolve_body_paragraph(model, payload);
    }
    if payload.get("paragraph").is_some() {
        let mut aliased = payload.clone();
        aliased["index"] = payload["paragraph"].clone();
        return resolve_body_paragraph(model, &aliased);
    }
    resolve_body_paragraph(model, payload)
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
    let mut seen = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format_error(format!("cannot read ZIP entry: {error}")))?;
        let name = entry.name().to_owned();
        seen.insert(name.clone());
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
    for (name, replacement) in replacements {
        if seen.contains(name) {
            continue;
        }
        writer
            .start_file(name, SimpleFileOptions::default())
            .map_err(|error| format_error(format!("cannot start added ZIP entry: {error}")))?;
        writer
            .write_all(replacement)
            .map_err(|error| format_error(format!("cannot write added ZIP entry: {error}")))?;
    }
    writer
        .finish()
        .map_err(|error| format_error(format!("cannot finish DOCX package: {error}")))
        .map(|cursor| cursor.into_inner())
}

fn required_bool(payload: &serde_json::Value, key: &str) -> Result<bool> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| format_error(format!("`{key}` boolean is required")))
}

fn optional_positive_f64(payload: &serde_json::Value, key: &str) -> Result<Option<f64>> {
    match payload.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.0)
            .map(Some)
            .ok_or_else(|| format_error(format!("`{key}` must be a positive number"))),
    }
}

fn optional_non_negative_f64(payload: &serde_json::Value, key: &str) -> Result<Option<f64>> {
    match payload.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .map(Some)
            .ok_or_else(|| format_error(format!("`{key}` must be a non-negative number"))),
    }
}

fn optional_str<'a>(payload: &'a serde_json::Value, key: &str) -> Result<Option<&'a str>> {
    match payload.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(Some)
            .ok_or_else(|| format_error(format!("`{key}` must be a non-empty string"))),
    }
}

fn optional_vert_align<'a>(payload: &'a serde_json::Value, key: &str) -> Result<Option<&'a str>> {
    match payload.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let text = value
                .as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| format_error(format!("`{key}` must be a non-empty string")))?;
            match text {
                "superscript" | "subscript" => Ok(Some(text)),
                _ => Err(format_error(format!(
                    "`{key}` must be superscript, subscript, or null"
                ))),
            }
        }
    }
}

fn optional_caps<'a>(payload: &'a serde_json::Value, key: &str) -> Result<Option<&'a str>> {
    match payload.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let text = value
                .as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| format_error(format!("`{key}` must be a non-empty string")))?;
            match text {
                "small" | "all" => Ok(Some(text)),
                _ => Err(format_error(format!("`{key}` must be small, all, or null"))),
            }
        }
    }
}

fn optional_hyperlink_url<'a>(
    payload: &'a serde_json::Value,
    key: &str,
) -> Result<Option<&'a str>> {
    match payload.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let text = value
                .as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| format_error(format!("`{key}` must be a non-empty string")))?;
            if is_allowed_external_url(text) {
                Ok(Some(text))
            } else {
                Err(format_error(format!(
                    "`{key}` must start with http://, https://, or mailto:"
                )))
            }
        }
    }
}

fn is_allowed_external_url(url: &str) -> bool {
    starts_with_ignore_ascii_case(url, "http://")
        || starts_with_ignore_ascii_case(url, "https://")
        || starts_with_ignore_ascii_case(url, "mailto:")
}

fn starts_with_ignore_ascii_case(haystack: &str, prefix: &str) -> bool {
    haystack.len() >= prefix.len()
        && haystack.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn optional_srgb_color(payload: &serde_json::Value, key: &str) -> Result<Option<String>> {
    match payload.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let raw = value
                .as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| format_error(format!("`{key}` must be a non-empty string")))?;
            Ok(Some(normalize_srgb_color(raw)?))
        }
    }
}

fn optional_highlight_color(payload: &serde_json::Value, key: &str) -> Result<Option<String>> {
    match payload.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let raw = value
                .as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| format_error(format!("`{key}` must be a non-empty string")))?;
            Ok(Some(normalize_highlight_color(raw)?))
        }
    }
}

fn normalize_srgb_color(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_start_matches('#');
    if trimmed.len() != 6 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format_error(
            "`color` must be an RRGGBB hex string (optional leading #)",
        ));
    }
    Ok(trimmed.to_ascii_uppercase())
}

const HIGHLIGHT_COLORS: &[&str] = &[
    "yellow",
    "green",
    "cyan",
    "magenta",
    "blue",
    "red",
    "darkBlue",
    "darkCyan",
    "darkGreen",
    "darkMagenta",
    "darkRed",
    "darkYellow",
    "darkGray",
    "lightGray",
    "black",
    "none",
];

fn normalize_highlight_color(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return Ok("none".into());
    }
    for candidate in HIGHLIGHT_COLORS {
        if trimmed.eq_ignore_ascii_case(candidate) {
            return Ok((*candidate).to_owned());
        }
    }
    Err(format_error(
        "`color` must be a Word highlight name (yellow, green, cyan, magenta, blue, red, darkBlue, darkCyan, darkGreen, darkMagenta, darkRed, darkYellow, darkGray, lightGray, black, or none)",
    ))
}

fn format_size_pt(value: f64) -> String {
    if (value - value.round()).abs() < f64::EPSILON {
        format!("{}", value.round() as i64)
    } else {
        format!("{value}")
    }
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
