use std::io::{Cursor, Read};
use std::path::Path;

use dotall_core::{DotallError, Result};
use quick_xml::Reader;
use quick_xml::events::Event;
use zip::ZipArchive;

use crate::FORMAT_ID;
use crate::ids;
use crate::model::{DocumentModel, HeaderFooterParagraphModel, ParagraphModel, SCHEMA_VERSION};

pub fn parse_document(source: &Path) -> Result<DocumentModel> {
    let bytes = std::fs::read(source).map_err(|error| DotallError::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    parse_document_bytes(&bytes)
}

pub fn parse_document_bytes(package: &[u8]) -> Result<DocumentModel> {
    let source_hash = blake3::hash(package).to_hex().to_string();
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(format!("invalid DOCX package: {error}")))?;
    let document_xml = zip_entry(&mut archive, "word/document.xml")?;
    let (paragraphs, table_count) = parse_body(&document_xml)?;
    let header_names = list_story_parts(&mut archive, "word/header")?;
    let footer_names = list_story_parts(&mut archive, "word/footer")?;
    let header_paragraphs = parse_story_parts(&mut archive, &header_names, "header")?;
    let footer_paragraphs = parse_story_parts(&mut archive, &footer_names, "footer")?;
    Ok(DocumentModel {
        document_id: ids::document_id(&source_hash, SCHEMA_VERSION),
        paragraphs,
        header_paragraphs,
        footer_paragraphs,
        skipped_tables: false,
        table_count,
    })
}

fn list_story_parts(archive: &mut ZipArchive<Cursor<&[u8]>>, prefix: &str) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format_error(format!("cannot list ZIP entry: {error}")))?;
        let name = entry.name().to_owned();
        if is_story_part(&name, prefix) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

fn is_story_part(name: &str, prefix: &str) -> bool {
    let Some(rest) = name.strip_prefix(prefix) else {
        return false;
    };
    let Some(stem) = rest.strip_suffix(".xml") else {
        return false;
    };
    !stem.is_empty() && stem.chars().all(|ch| ch.is_ascii_digit())
}

fn parse_story_parts(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    names: &[String],
    kind: &str,
) -> Result<Vec<HeaderFooterParagraphModel>> {
    let mut paragraphs = Vec::new();
    for name in names {
        let xml = zip_entry(archive, name)?;
        let part = part_stem(name);
        let part_paragraphs = parse_story_xml(&xml, kind, &part)?;
        paragraphs.extend(part_paragraphs);
    }
    Ok(paragraphs)
}

fn part_stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(name)
        .to_owned()
}

fn zip_entry(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let mut entry = archive
        .by_name(name)
        .map_err(|error| format_error(format!("missing `{name}`: {error}")))?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| format_error(format!("cannot read `{name}`: {error}")))?;
    Ok(bytes)
}

fn parse_body(xml: &[u8]) -> Result<(Vec<ParagraphModel>, u32)> {
    let parsed = parse_paragraphs(xml)?;
    let mut paragraphs = Vec::with_capacity(parsed.paragraphs.len());
    for (index, draft) in parsed.paragraphs.into_iter().enumerate() {
        let index = index as u32;
        paragraphs.push(ParagraphModel {
            element_id: ids::paragraph_id(index, &draft.text, SCHEMA_VERSION),
            index,
            outline_level: draft.outline_level,
            style_id: draft.style_id,
            text: draft.text,
            editable: draft.editable,
            in_table: draft.in_table,
        });
    }
    Ok((paragraphs, parsed.table_count))
}

fn parse_story_xml(xml: &[u8], kind: &str, part: &str) -> Result<Vec<HeaderFooterParagraphModel>> {
    let parsed = parse_paragraphs(xml)?;
    let mut paragraphs = Vec::with_capacity(parsed.paragraphs.len());
    for (index, draft) in parsed.paragraphs.into_iter().enumerate() {
        let index = index as u32;
        paragraphs.push(HeaderFooterParagraphModel {
            element_id: ids::header_footer_paragraph_id(
                kind,
                part,
                index,
                &draft.text,
                SCHEMA_VERSION,
            ),
            part: part.to_owned(),
            index,
            outline_level: draft.outline_level,
            style_id: draft.style_id,
            text: draft.text,
            editable: draft.editable,
        });
    }
    Ok(paragraphs)
}

struct DraftParagraph {
    outline_level: Option<u32>,
    style_id: Option<String>,
    text: String,
    editable: bool,
    in_table: bool,
}

struct ParsedParagraphs {
    paragraphs: Vec<DraftParagraph>,
    table_count: u32,
}

fn parse_paragraphs(xml: &[u8]) -> Result<ParsedParagraphs> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut paragraphs = Vec::new();
    let mut table_count = 0u32;
    let mut table_depth = 0u32;
    let mut in_paragraph = false;
    let mut texts = Vec::new();
    let mut style_id = None;
    let mut outline_level = None;
    let mut editable = true;

    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(format!("invalid document XML: {error}")))?
        {
            Event::Start(tag) if tag.local_name().as_ref() == b"tbl" => {
                if table_depth == 0 {
                    table_count += 1;
                }
                table_depth += 1;
            }
            Event::End(tag) if tag.local_name().as_ref() == b"tbl" => {
                table_depth = table_depth.saturating_sub(1);
            }
            Event::Start(tag) if tag.local_name().as_ref() == b"p" => {
                in_paragraph = true;
                texts.clear();
                style_id = None;
                outline_level = None;
                editable = true;
            }
            Event::Start(tag)
                if in_paragraph
                    && matches!(
                        tag.local_name().as_ref(),
                        b"del" | b"ins" | b"sdt" | b"fldChar" | b"instrText"
                    ) =>
            {
                editable = false;
            }
            Event::Empty(tag) | Event::Start(tag)
                if in_paragraph && tag.local_name().as_ref() == b"pStyle" =>
            {
                style_id = attribute_val(&tag, b"val")?;
            }
            Event::Empty(tag) | Event::Start(tag)
                if in_paragraph && tag.local_name().as_ref() == b"outlineLvl" =>
            {
                outline_level = attribute_val(&tag, b"val")?.and_then(|value| value.parse().ok());
            }
            Event::Start(tag) if in_paragraph && tag.local_name().as_ref() == b"t" => {
                let text = reader
                    .read_text(tag.name())
                    .map_err(|error| format_error(format!("invalid w:t: {error}")))?;
                texts.push(decode_text(text));
            }
            Event::End(tag) if tag.local_name().as_ref() == b"p" && in_paragraph => {
                paragraphs.push(DraftParagraph {
                    outline_level,
                    style_id: style_id.take(),
                    text: texts.concat(),
                    editable,
                    in_table: table_depth > 0,
                });
                in_paragraph = false;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(ParsedParagraphs {
        paragraphs,
        table_count,
    })
}

fn attribute_val(tag: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Result<Option<String>> {
    for attribute in tag.attributes() {
        let attribute =
            attribute.map_err(|error| format_error(format!("invalid attribute: {error}")))?;
        if attribute.key.local_name().as_ref() == name {
            return Ok(Some(String::from_utf8_lossy(&attribute.value).into_owned()));
        }
    }
    Ok(None)
}

fn decode_text(text: quick_xml::events::BytesText<'_>) -> String {
    let decoded = text
        .decode()
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(text.as_ref()).into_owned());
    quick_xml::escape::unescape(&decoded)
        .map(|value| value.into_owned())
        .unwrap_or(decoded)
}

fn format_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<docx>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_document_bytes;
    use crate::fixture::{header_footer_docx, minimal_docx, table_docx};

    #[test]
    fn parses_body_paragraphs_in_order() {
        let model = parse_document_bytes(&minimal_docx()).expect("parse");
        assert_eq!(model.paragraphs.len(), 2);
        assert_eq!(model.paragraphs[0].text, "Alpha");
        assert_eq!(model.paragraphs[1].text, "Beta");
        assert_eq!(model.paragraphs[0].style_id.as_deref(), Some("Heading1"));
        assert_eq!(model.table_count, 0);
        assert!(!model.skipped_tables);
        assert!(model.header_paragraphs.is_empty());
    }

    #[test]
    fn parses_table_cell_paragraphs_in_document_order() {
        let model = parse_document_bytes(&table_docx()).expect("parse");
        assert_eq!(model.paragraphs.len(), 3);
        assert_eq!(model.paragraphs[0].text, "Intro");
        assert_eq!(model.paragraphs[1].text, "CellA");
        assert_eq!(model.paragraphs[2].text, "CellB");
        assert_eq!(model.table_count, 1);
        assert!(!model.skipped_tables);
        assert_eq!(model.header_paragraphs.len(), 1);
        assert_eq!(model.header_paragraphs[0].part, "header1");
        assert_eq!(model.header_paragraphs[0].text, "HeaderOnly");
    }

    #[test]
    fn parses_header_and_footer_parts() {
        let model = parse_document_bytes(&header_footer_docx()).expect("parse");
        assert_eq!(model.header_paragraphs.len(), 1);
        assert_eq!(model.footer_paragraphs.len(), 1);
        assert_eq!(model.footer_paragraphs[0].part, "footer1");
        assert_eq!(model.footer_paragraphs[0].text, "FooterOnly");
    }
}
