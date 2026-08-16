use std::io::{Cursor, Read};
use std::path::Path;

use dotall_core::{DotallError, Result};
use quick_xml::Reader;
use quick_xml::events::Event;
use zip::ZipArchive;

use crate::FORMAT_ID;
use crate::ids;
use crate::model::{DocumentModel, ParagraphModel, SCHEMA_VERSION};

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
    let (paragraphs, skipped_tables) = parse_body(&document_xml)?;
    Ok(DocumentModel {
        document_id: ids::document_id(&source_hash, SCHEMA_VERSION),
        paragraphs,
        skipped_tables,
    })
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

fn parse_body(xml: &[u8]) -> Result<(Vec<ParagraphModel>, bool)> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut paragraphs = Vec::new();
    let mut skipped_tables = false;
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
                table_depth += 1;
                skipped_tables = true;
            }
            Event::End(tag) if tag.local_name().as_ref() == b"tbl" => {
                table_depth = table_depth.saturating_sub(1);
            }
            Event::Start(tag) if tag.local_name().as_ref() == b"p" && table_depth == 0 => {
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
                let text = texts.concat();
                let index = paragraphs.len() as u32;
                paragraphs.push(ParagraphModel {
                    element_id: ids::paragraph_id(index, &text, SCHEMA_VERSION),
                    index,
                    outline_level,
                    style_id: style_id.take(),
                    text,
                    editable,
                });
                in_paragraph = false;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok((paragraphs, skipped_tables))
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
    use crate::fixture::minimal_docx;

    #[test]
    fn parses_body_paragraphs_in_order() {
        let model = parse_document_bytes(&minimal_docx()).expect("parse");
        assert_eq!(model.paragraphs.len(), 2);
        assert_eq!(model.paragraphs[0].text, "Alpha");
        assert_eq!(model.paragraphs[1].text, "Beta");
        assert_eq!(model.paragraphs[0].style_id.as_deref(), Some("Heading1"));
    }
}
