use std::collections::{BTreeMap, BTreeSet};

use dotall_core::{DotallError, Result};
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::FORMAT_ID;
use crate::edits::{EditableValue, XlsxEditOp};

pub(super) fn patch(xml: &[u8], operations: &[XlsxEditOp]) -> Result<Vec<u8>> {
    let edits = operations
        .iter()
        .map(|operation| match operation {
            XlsxEditOp::SetCellValue { address, .. }
            | XlsxEditOp::SetCellFormula { address, .. } => (address.as_str(), operation),
        })
        .collect::<BTreeMap<_, _>>();
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut replacements = Vec::new();
    let mut found = BTreeSet::new();

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| writer_error(format!("invalid worksheet XML: {error}")))?;
        match event {
            Event::Start(start) if start.name().as_ref() == b"c" => {
                let end = reader.buffer_position() as usize;
                let start_offset = end.saturating_sub(start.len() + 2);
                let address = cell_address(&start)?;
                let start_tag = start.to_owned();
                let element_name = start.name().as_ref().to_vec();
                let mut end_buffer = Vec::new();
                reader
                    .read_to_end_into(quick_xml::name::QName(&element_name), &mut end_buffer)
                    .map_err(|error| writer_error(format!("invalid cell XML: {error}")))?;
                let end_offset = reader.buffer_position() as usize;
                if let Some(operation) = edits.get(address.as_str()) {
                    replacements.push((
                        start_offset,
                        end_offset,
                        render_cell(&start_tag, operation)?,
                    ));
                    found.insert(address);
                }
            }
            Event::Empty(start) if start.name().as_ref() == b"c" => {
                let end = reader.buffer_position() as usize;
                let start_offset = end.saturating_sub(start.len() + 3);
                let address = cell_address(&start)?;
                if let Some(operation) = edits.get(address.as_str()) {
                    replacements.push((
                        start_offset,
                        end,
                        render_cell(&start.to_owned(), operation)?,
                    ));
                    found.insert(address);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    let mut patched = Vec::with_capacity(xml.len());
    let mut cursor = 0;
    for (start, end, replacement) in replacements {
        patched.extend_from_slice(&xml[cursor..start]);
        patched.extend_from_slice(replacement.as_bytes());
        cursor = end;
    }
    patched.extend_from_slice(&xml[cursor..]);

    let missing = operations
        .iter()
        .filter(|operation| {
            let address = match operation {
                XlsxEditOp::SetCellValue { address, .. }
                | XlsxEditOp::SetCellFormula { address, .. } => address,
            };
            !found.contains(address)
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(patched);
    }

    let mut rows = BTreeMap::<u32, String>::new();
    for operation in missing {
        let address = match operation {
            XlsxEditOp::SetCellValue { address, .. }
            | XlsxEditOp::SetCellFormula { address, .. } => address,
        };
        let row = row_number(address)?;
        rows.entry(row)
            .or_default()
            .push_str(&render_new_cell(operation)?);
    }
    let mut new_rows = String::new();
    for (row, cells) in rows {
        if let Some(row_start) = find_row(&patched, row) {
            let row_end = patched[row_start..]
                .windows(b"</row>".len())
                .position(|window| window == b"</row>")
                .map(|offset| row_start + offset)
                .ok_or_else(|| writer_error(format!("worksheet row `{row}` is not closed")))?;
            patched.splice(row_end..row_end, cells.bytes());
        } else {
            new_rows.push_str(&format!(r#"<row r="{row}">{cells}</row>"#));
        }
    }
    if !new_rows.is_empty() {
        let insertion = b"</sheetData>";
        let position = patched
            .windows(insertion.len())
            .position(|window| window == insertion)
            .ok_or_else(|| writer_error("worksheet is missing sheetData"))?;
        patched.splice(position..position, new_rows.bytes());
    }
    Ok(patched)
}

fn cell_address(start: &quick_xml::events::BytesStart<'_>) -> Result<String> {
    start
        .attributes()
        .filter_map(|attribute| attribute.ok())
        .find(|attribute| attribute.key.as_ref() == b"r")
        .map(|attribute| String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
        .ok_or_else(|| writer_error("worksheet cell is missing its address"))
}

fn render_cell(
    start: &quick_xml::events::BytesStart<'_>,
    operation: &XlsxEditOp,
) -> Result<String> {
    let address = cell_address(start)?;
    let mut attributes = Vec::new();
    for attribute in start.attributes().flatten() {
        let key = attribute.key.as_ref();
        if key != b"r" && key != b"t" {
            attributes.push(format!(
                r#" {}="{}""#,
                String::from_utf8_lossy(key),
                String::from_utf8_lossy(attribute.value.as_ref())
            ));
        }
    }
    render_cell_parts(&address, &attributes.concat(), operation)
}

fn render_new_cell(operation: &XlsxEditOp) -> Result<String> {
    let address = match operation {
        XlsxEditOp::SetCellValue { address, .. } | XlsxEditOp::SetCellFormula { address, .. } => {
            address
        }
    };
    render_cell_parts(address, "", operation)
}

fn row_number(address: &str) -> Result<u32> {
    address
        .chars()
        .skip_while(|character| character.is_ascii_alphabetic())
        .collect::<String>()
        .parse()
        .map_err(|_| writer_error(format!("invalid cell address `{address}`")))
}

fn find_row(xml: &[u8], row: u32) -> Option<usize> {
    let prefix = format!(r#"<row r="{row}""#);
    xml.windows(prefix.len())
        .position(|window| window == prefix.as_bytes())
}

fn render_cell_parts(
    address: &str,
    preserved_attributes: &str,
    operation: &XlsxEditOp,
) -> Result<String> {
    match operation {
        XlsxEditOp::SetCellValue { value, .. } => match value {
            EditableValue::String(value) => Ok(format!(
                r#"<c r="{address}" t="inlineStr"{preserved_attributes}><is><t>{}</t></is></c>"#,
                escape(value)
            )),
            EditableValue::Number(value) => Ok(format!(
                r#"<c r="{address}"{preserved_attributes}><v>{value}</v></c>"#
            )),
            EditableValue::Boolean(value) => Ok(format!(
                r#"<c r="{address}" t="b"{preserved_attributes}><v>{}</v></c>"#,
                u8::from(*value)
            )),
            EditableValue::Blank => Ok(format!(r#"<c r="{address}"{preserved_attributes}/>"#)),
        },
        XlsxEditOp::SetCellFormula { formula, .. } => {
            let formula = formula.strip_prefix('=').unwrap_or(formula);
            Ok(format!(
                r#"<c r="{address}"{preserved_attributes}><f>{}</f><v></v></c>"#,
                escape(formula)
            ))
        }
    }
}

fn escape(value: &str) -> String {
    quick_xml::escape::escape(value).into_owned()
}

fn writer_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx worksheet writer>".into(),
        message: message.into(),
    }
}
