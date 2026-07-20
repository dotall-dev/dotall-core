use std::collections::BTreeMap;

use dotall_core::{DotallError, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::FORMAT_ID;

pub(super) struct SharedStringsPatch {
    pub(super) bytes: Vec<u8>,
    pub(super) indices: BTreeMap<String, usize>,
}

pub(super) fn patch(
    xml: &[u8],
    requested: impl IntoIterator<Item = String>,
    count_delta: u64,
) -> Result<SharedStringsPatch> {
    let (count, unique_count, strings) = parse(xml)?;
    let mut indices = strings
        .into_iter()
        .enumerate()
        .map(|(index, value)| (value, index))
        .collect::<BTreeMap<_, _>>();
    let mut appended = Vec::new();
    for value in requested {
        if !indices.contains_key(&value) {
            let index = indices.len();
            indices.insert(value.clone(), index);
            appended.push(value);
        }
    }

    let opening_end = opening_tag_end(xml)?;
    let closing_start = xml
        .windows(b"</sst>".len())
        .rposition(|window| window == b"</sst>")
        .ok_or_else(|| writer_error("shared strings XML is missing its closing sst tag"))?;
    let mut bytes = render_opening_tag(
        xml,
        opening_end,
        count + count_delta,
        unique_count + appended.len() as u64,
    )?;
    bytes.extend_from_slice(&xml[opening_end..closing_start]);
    for value in appended {
        bytes.extend_from_slice(render_string_item(&value).as_bytes());
    }
    bytes.extend_from_slice(&xml[closing_start..]);

    Ok(SharedStringsPatch { bytes, indices })
}

fn parse(xml: &[u8]) -> Result<(u64, u64, Vec<String>)> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut count = None;
    let mut unique_count = None;
    let mut strings = Vec::new();
    let mut current = None;
    let mut in_text = false;

    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| writer_error(format!("invalid shared strings XML: {error}")))?
        {
            Event::Start(element) if element.name().as_ref() == b"sst" => {
                for attribute in element.attributes().flatten() {
                    match attribute.key.as_ref() {
                        b"count" => count = parse_count(attribute.value.as_ref(), "count")?,
                        b"uniqueCount" => {
                            unique_count = parse_count(attribute.value.as_ref(), "uniqueCount")?
                        }
                        _ => {}
                    }
                }
            }
            Event::Start(element) if element.name().as_ref() == b"si" => {
                current = Some(String::new())
            }
            Event::End(element) if element.name().as_ref() == b"si" => {
                strings.push(current.take().unwrap_or_default());
            }
            Event::Start(element) if element.name().as_ref() == b"t" => in_text = true,
            Event::End(element) if element.name().as_ref() == b"t" => in_text = false,
            Event::Text(text) if in_text => {
                if let Some(value) = &mut current {
                    let decoded = text
                        .xml10_content()
                        .map_err(|error| writer_error(format!("invalid shared string: {error}")))?;
                    value.push_str(&quick_xml::escape::unescape(&decoded).map_err(|error| {
                        writer_error(format!("invalid shared string: {error}"))
                    })?);
                }
            }
            Event::CData(text) if in_text => {
                if let Some(value) = &mut current {
                    value.push_str(&text.decode().map_err(|error| {
                        writer_error(format!("invalid shared string: {error}"))
                    })?);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    Ok((
        count.unwrap_or(strings.len() as u64),
        unique_count.unwrap_or(strings.len() as u64),
        strings,
    ))
}

fn parse_count(value: &[u8], name: &str) -> Result<Option<u64>> {
    String::from_utf8_lossy(value)
        .parse()
        .map(Some)
        .map_err(|_| writer_error(format!("shared strings `{name}` attribute is invalid")))
}

fn opening_tag_end(xml: &[u8]) -> Result<usize> {
    let start = xml
        .windows(b"<sst".len())
        .position(|window| window == b"<sst")
        .ok_or_else(|| writer_error("shared strings XML is missing its sst tag"))?;
    let mut quote = None;
    for (offset, byte) in xml[start..].iter().enumerate() {
        match (quote, byte) {
            (None, b'"' | b'\'') => quote = Some(*byte),
            (Some(delimiter), byte) if *byte == delimiter => quote = None,
            (None, b'>') => return Ok(start + offset + 1),
            _ => {}
        }
    }
    Err(writer_error(
        "shared strings XML has an unterminated sst tag",
    ))
}

fn render_opening_tag(
    xml: &[u8],
    opening_end: usize,
    count: u64,
    unique_count: u64,
) -> Result<Vec<u8>> {
    let mut reader = Reader::from_reader(&xml[..opening_end]);
    let mut buffer = Vec::new();
    let element = loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| writer_error(format!("invalid shared strings XML: {error}")))?
        {
            Event::Start(element) if element.name().as_ref() == b"sst" => break element.to_owned(),
            Event::Eof => return Err(writer_error("shared strings XML is missing its sst tag")),
            _ => {}
        }
        buffer.clear();
    };

    let mut rendered = String::from("<sst");
    let mut has_count = false;
    let mut has_unique_count = false;
    for attribute in element.attributes().flatten() {
        let key = String::from_utf8_lossy(attribute.key.as_ref());
        let value = match attribute.key.as_ref() {
            b"count" => {
                has_count = true;
                count.to_string()
            }
            b"uniqueCount" => {
                has_unique_count = true;
                unique_count.to_string()
            }
            _ => String::from_utf8_lossy(attribute.value.as_ref()).into_owned(),
        };
        rendered.push_str(&format!(r#" {key}="{}""#, escape_attribute(&value)));
    }
    if !has_count {
        rendered.push_str(&format!(r#" count="{count}""#));
    }
    if !has_unique_count {
        rendered.push_str(&format!(r#" uniqueCount="{unique_count}""#));
    }
    rendered.push('>');
    Ok(rendered.into_bytes())
}

fn render_string_item(value: &str) -> String {
    let preserve = value.starts_with(char::is_whitespace) || value.ends_with(char::is_whitespace);
    if preserve {
        format!(r#"<si><t xml:space="preserve">{}</t></si>"#, escape(value))
    } else {
        format!(r#"<si><t>{}</t></si>"#, escape(value))
    }
}

fn escape(value: &str) -> String {
    quick_xml::escape::escape(value).into_owned()
}

fn escape_attribute(value: &str) -> String {
    quick_xml::escape::escape(value).into_owned()
}

fn writer_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx shared strings writer>".into(),
        message: message.into(),
    }
}
