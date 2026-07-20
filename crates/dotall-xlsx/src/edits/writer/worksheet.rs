use std::collections::{BTreeMap, BTreeSet};

use dotall_core::{DotallError, Result};
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::FORMAT_ID;
use crate::edits::{EditableValue, XlsxEditOp};

pub(super) fn patch(
    xml: &[u8],
    operations: &[XlsxEditOp],
    shared_string_indices: Option<&BTreeMap<String, usize>>,
) -> Result<Vec<u8>> {
    let edits = operations
        .iter()
        .map(|operation| match operation {
            XlsxEditOp::SetCellValue { address, .. }
            | XlsxEditOp::SetCellFormula { address, .. } => (address.as_str(), operation),
            XlsxEditOp::InsertRow { .. }
            | XlsxEditOp::DeleteRow { .. }
            | XlsxEditOp::SetRange { .. } => {
                unreachable!("structural operations return before worksheet patching")
            }
        })
        .collect::<BTreeMap<_, _>>();
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut replacements = Vec::new();
    let mut found = BTreeSet::new();
    let mut protected_formula_ranges = Vec::new();

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
                if let Some((formula_type, range)) =
                    unsupported_formula(&xml[start_offset..end_offset])?
                {
                    if let Some(range) = range {
                        protected_formula_ranges.push(range);
                    }
                    if edits.contains_key(address.as_str()) {
                        return Err(writer_error(format!(
                            "cannot edit cell `{address}` containing a {formula_type} formula"
                        )));
                    }
                }
                if let Some(operation) = edits.get(address.as_str()) {
                    replacements.push((
                        start_offset,
                        end_offset,
                        render_cell(&start_tag, operation, shared_string_indices)?,
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
                        render_cell(&start.to_owned(), operation, shared_string_indices)?,
                    ));
                    found.insert(address);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    for address in edits.keys() {
        if protected_formula_ranges
            .iter()
            .any(|range| address_in_range(address, range))
        {
            return Err(writer_error(format!(
                "cannot edit cell `{address}` because it belongs to a shared, array, or data table formula range"
            )));
        }
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
                XlsxEditOp::InsertRow { .. }
                | XlsxEditOp::DeleteRow { .. }
                | XlsxEditOp::SetRange { .. } => {
                    unreachable!("structural operations return before worksheet patching")
                }
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
            XlsxEditOp::InsertRow { .. }
            | XlsxEditOp::DeleteRow { .. }
            | XlsxEditOp::SetRange { .. } => {
                unreachable!("structural operations return before worksheet patching")
            }
        };
        let row = row_number(address)?;
        rows.entry(row)
            .or_default()
            .push_str(&render_new_cell(operation, shared_string_indices)?);
    }
    let mut new_rows = String::new();
    for (row, cells) in rows {
        if let Some(row_end) = find_row_end(&patched, row)? {
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

pub(super) fn shared_string_count_delta(xml: &[u8], operations: &[XlsxEditOp]) -> Result<u64> {
    let string_edits = operations
        .iter()
        .filter_map(|operation| match operation {
            XlsxEditOp::SetCellValue {
                address,
                value: EditableValue::String(_),
                ..
            } => Some(address.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let requested_count = string_edits.len() as u64;
    let mut remaining = string_edits;
    let mut existing_shared_strings = 0_u64;
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| writer_error(format!("invalid worksheet XML: {error}")))?
        {
            Event::Start(cell) | Event::Empty(cell) if cell.name().as_ref() == b"c" => {
                let address = cell_address(&cell)?;
                if remaining.remove(address.as_str()) && cell_is_shared_string(&cell) {
                    existing_shared_strings += 1;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    Ok(requested_count - existing_shared_strings)
}

fn cell_is_shared_string(start: &quick_xml::events::BytesStart<'_>) -> bool {
    start
        .attributes()
        .filter_map(|attribute| attribute.ok())
        .find(|attribute| attribute.key.as_ref() == b"t")
        .is_some_and(|attribute| attribute.value.as_ref() == b"s")
}

fn render_cell(
    start: &quick_xml::events::BytesStart<'_>,
    operation: &XlsxEditOp,
    shared_string_indices: Option<&BTreeMap<String, usize>>,
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
    render_cell_parts(
        &address,
        &attributes.concat(),
        operation,
        shared_string_indices,
    )
}

fn render_new_cell(
    operation: &XlsxEditOp,
    shared_string_indices: Option<&BTreeMap<String, usize>>,
) -> Result<String> {
    let address = match operation {
        XlsxEditOp::SetCellValue { address, .. } | XlsxEditOp::SetCellFormula { address, .. } => {
            address
        }
        XlsxEditOp::InsertRow { .. }
        | XlsxEditOp::DeleteRow { .. }
        | XlsxEditOp::SetRange { .. } => {
            unreachable!("structural operations return before worksheet patching")
        }
    };
    render_cell_parts(address, "", operation, shared_string_indices)
}

fn row_number(address: &str) -> Result<u32> {
    address
        .chars()
        .skip_while(|character| character.is_ascii_alphabetic())
        .collect::<String>()
        .parse()
        .map_err(|_| writer_error(format!("invalid cell address `{address}`")))
}

fn find_row_end(xml: &[u8], target_row: u32) -> Result<Option<usize>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut matching_row_open = false;

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| writer_error(format!("invalid worksheet XML: {error}")))?;
        match event {
            Event::Start(start) if start.name().as_ref() == b"row" => {
                matching_row_open = row_reference(&start)? == target_row;
            }
            Event::Empty(start) if start.name().as_ref() == b"row" => {
                if row_reference(&start)? == target_row {
                    return Err(writer_error(format!(
                        "cannot add a cell to self-closing worksheet row `{target_row}`"
                    )));
                }
            }
            Event::End(end) if end.name().as_ref() == b"row" && matching_row_open => {
                let end_offset = reader.buffer_position() as usize;
                return Ok(Some(end_offset - end.name().as_ref().len() - 3));
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
        buffer.clear();
    }
}

fn row_reference(start: &quick_xml::events::BytesStart<'_>) -> Result<u32> {
    start
        .attributes()
        .filter_map(|attribute| attribute.ok())
        .find(|attribute| attribute.key.as_ref() == b"r")
        .map(|attribute| String::from_utf8_lossy(attribute.value.as_ref()).parse())
        .ok_or_else(|| writer_error("worksheet row is missing its reference"))?
        .map_err(|_| writer_error("worksheet row has an invalid reference"))
}

fn unsupported_formula(xml: &[u8]) -> Result<Option<(String, Option<String>)>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| writer_error(format!("invalid cell XML: {error}")))?
        {
            Event::Start(start) | Event::Empty(start) if start.name().as_ref() == b"f" => {
                let mut formula_type = None;
                let mut range = None;
                for attribute in start.attributes().flatten() {
                    match attribute.key.as_ref() {
                        b"t" => {
                            formula_type =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        b"ref" => {
                            range =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        _ => {}
                    }
                }
                if let Some(formula_type @ ("shared" | "array" | "dataTable")) =
                    formula_type.as_deref()
                {
                    return Ok(Some((formula_type.to_owned(), range)));
                }
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
        buffer.clear();
    }
}

fn address_in_range(address: &str, range: &str) -> bool {
    let Some((start, end)) = range.split_once(':') else {
        return address == range;
    };
    let Some((column, row)) = split_address(address) else {
        return false;
    };
    let (Some((start_column, start_row)), Some((end_column, end_row))) =
        (split_address(start), split_address(end))
    else {
        return false;
    };
    column >= start_column && column <= end_column && row >= start_row && row <= end_row
}

fn split_address(address: &str) -> Option<(u32, u32)> {
    let letters = address
        .chars()
        .take_while(|character| character.is_ascii_alphabetic())
        .collect::<String>();
    let row = address.get(letters.len()..)?.parse().ok()?;
    let column = letters.chars().try_fold(0_u32, |column, character| {
        Some(column.checked_mul(26)? + (character.to_ascii_uppercase() as u32 - 'A' as u32 + 1))
    })?;
    Some((column, row))
}

fn render_cell_parts(
    address: &str,
    preserved_attributes: &str,
    operation: &XlsxEditOp,
    shared_string_indices: Option<&BTreeMap<String, usize>>,
) -> Result<String> {
    match operation {
        XlsxEditOp::SetCellValue { value, .. } => match value {
            EditableValue::String(value) => match shared_string_indices {
                Some(indices) => {
                    let index = indices.get(value).ok_or_else(|| {
                        writer_error(format!(
                            "shared string index not found for cell `{address}`"
                        ))
                    })?;
                    Ok(format!(
                        r#"<c r="{address}" t="s"{preserved_attributes}><v>{index}</v></c>"#
                    ))
                }
                None => Ok(format!(
                    r#"<c r="{address}" t="inlineStr"{preserved_attributes}><is><t>{}</t></is></c>"#,
                    escape(value)
                )),
            },
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
        XlsxEditOp::InsertRow { .. }
        | XlsxEditOp::DeleteRow { .. }
        | XlsxEditOp::SetRange { .. } => {
            unreachable!("structural operations return before worksheet patching")
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
