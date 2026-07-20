use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;
use crate::edits::transform::{
    Axis, AxisChange, TransformResult, parse_range, transform_formula, transform_range,
};

pub(super) fn patch(
    xml: &[u8],
    formula_sheet: &str,
    edited_sheet: &str,
    change: AxisChange,
) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|utf8_error| error(format!("worksheet is not UTF-8: {utf8_error}")))?;
    if !formula_sheet.eq_ignore_ascii_case(edited_sheet) {
        return Ok(
            transform_formula_nodes(source, formula_sheet, edited_sheet, change).into_bytes(),
        );
    }
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("<row") {
        let start = cursor + relative;
        let name_end = start + "<row".len();
        if source
            .as_bytes()
            .get(name_end)
            .is_some_and(|byte| !matches!(*byte, b'>' | b'/' | b' ' | b'\t' | b'\r' | b'\n'))
        {
            output.push_str(&source[cursor..name_end]);
            cursor = name_end;
            continue;
        }
        output.push_str(&source[cursor..start]);
        let tag_end = source[start..]
            .find('>')
            .map(|i| start + i)
            .ok_or_else(|| error("unterminated row"))?;
        let tag = &source[start..=tag_end];
        let row = attribute(tag, "r")
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| error("worksheet row is missing r"))?;
        let end = if tag.ends_with("/>") {
            tag_end + 1
        } else {
            source[tag_end + 1..]
                .find("</row>")
                .map(|i| tag_end + 1 + i + 6)
                .ok_or_else(|| error("unterminated row"))?
        };
        let transformed = transform_row(row, change)?;
        if let Some(row) = transformed {
            let block = replace_attribute(&source[start..end], "r", &row.to_string());
            output.push_str(&transform_cell_addresses(&block, change)?);
        }
        cursor = end;
    }
    output.push_str(&source[cursor..]);
    let output = transform_formula_nodes(&output, formula_sheet, edited_sheet, change);
    transform_ranges(&output, change)
}

fn transform_row(row: u32, change: AxisChange) -> Result<Option<u32>> {
    match change {
        AxisChange::Insert {
            axis: Axis::Column, ..
        }
        | AxisChange::Delete {
            axis: Axis::Column, ..
        } => Ok(Some(row)),
        AxisChange::Insert {
            axis: Axis::Row,
            at,
            count,
        } if row >= at => row
            .checked_add(count)
            .map(Some)
            .ok_or_else(|| error("row insert exceeds Excel limit")),
        AxisChange::Insert {
            axis: Axis::Row, ..
        } => Ok(Some(row)),
        AxisChange::Delete {
            axis: Axis::Row,
            at,
            count,
        } => {
            let end = at
                .checked_add(count - 1)
                .ok_or_else(|| error("invalid row delete"))?;
            if row >= at && row <= end {
                Ok(None)
            } else if row > end {
                Ok(Some(row - count))
            } else {
                Ok(Some(row))
            }
        }
    }
}

fn transform_cell_addresses(block: &str, change: AxisChange) -> Result<String> {
    let mut output = String::with_capacity(block.len());
    let mut cursor = 0;
    while let Some(relative) = block[cursor..].find("<c") {
        let start = cursor + relative;
        output.push_str(&block[cursor..start]);
        let tag_end = block[start..]
            .find('>')
            .map(|i| start + i)
            .ok_or_else(|| error("unterminated cell"))?;
        let tag = &block[start..=tag_end];
        let address = attribute(tag, "r").ok_or_else(|| error("worksheet cell is missing r"))?;
        let cell = crate::edits::transform::parse_cell(address).map_err(error)?;
        match crate::edits::transform::transform_cell(&cell, change) {
            TransformResult::Kept(cell) => {
                output.push_str(&replace_attribute(tag, "r", &cell.to_string()))
            }
            TransformResult::Removed | TransformResult::RefError => {
                if !matches!(
                    change,
                    AxisChange::Delete {
                        axis: Axis::Column,
                        ..
                    }
                ) {
                    return Err(error("cell address transformation failed"));
                }
                cursor = if tag.ends_with("/>") {
                    tag_end + 1
                } else {
                    block[tag_end + 1..]
                        .find("</c>")
                        .map(|i| tag_end + 1 + i + 4)
                        .ok_or_else(|| error("unterminated cell"))?
                };
                continue;
            }
        }
        cursor = tag_end + 1;
    }
    output.push_str(&block[cursor..]);
    Ok(output)
}

fn transform_formula_nodes(
    xml: &str,
    formula_sheet: &str,
    edited_sheet: &str,
    change: AxisChange,
) -> String {
    let mut output = String::with_capacity(xml.len());
    let mut cursor = 0;
    while let Some(relative) = xml[cursor..].find("<f") {
        let start = cursor + relative;
        let Some(open_end) = xml[start..].find('>').map(|i| start + i) else {
            break;
        };
        let Some(close_relative) = xml[open_end + 1..].find("</f>") else {
            break;
        };
        let close = open_end + 1 + close_relative;
        output.push_str(&xml[cursor..open_end + 1]);
        output.push_str(&transform_formula(
            &xml[open_end + 1..close],
            formula_sheet,
            edited_sheet,
            change,
        ));
        output.push_str("</f>");
        cursor = close + 4;
    }
    output.push_str(&xml[cursor..]);
    output
}

fn transform_ranges(xml: &str, change: AxisChange) -> Result<Vec<u8>> {
    let mut output = String::with_capacity(xml.len());
    let mut cursor = 0;
    while let Some(relative) = xml[cursor..].find("<mergeCell") {
        let start = cursor + relative;
        output.push_str(&xml[cursor..start]);
        let end = xml[start..]
            .find('>')
            .map(|i| start + i)
            .ok_or_else(|| error("unterminated mergeCell"))?;
        let tag = &xml[start..=end];
        let transformed = attribute(tag, "ref")
            .and_then(|reference| parse_range(reference).ok())
            .map(|range| transform_range(&range, change));
        match transformed {
            Some(TransformResult::Kept(range)) => {
                output.push_str(&replace_attribute(tag, "ref", &range.to_string()))
            }
            Some(TransformResult::Removed | TransformResult::RefError) => {}
            None => output.push_str(tag),
        }
        cursor = end + 1;
    }
    output.push_str(&xml[cursor..]);
    Ok(output.into_bytes())
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

fn replace_attribute(tag: &str, name: &str, value: &str) -> String {
    let needle = format!(" {name}=\"");
    let Some(start) = tag.find(&needle).map(|i| i + needle.len()) else {
        return tag.into();
    };
    let Some(end) = tag[start..].find('"').map(|i| start + i) else {
        return tag.into();
    };
    format!("{}{}{}", &tag[..start], value, &tag[end..])
}

fn error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx structural writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_scanner_skips_row_breaks() {
        let xml = br#"<worksheet><sheetData><row r="2"><c r="A2"><v>1</v></c></row></sheetData><rowBreaks count="1"><brk id="1"/></rowBreaks></worksheet>"#;

        let patched = patch(
            xml,
            "Inputs",
            "Inputs",
            AxisChange::Insert {
                axis: Axis::Row,
                at: 2,
                count: 1,
            },
        )
        .expect("row breaks are not worksheet rows");

        let patched = String::from_utf8(patched).expect("UTF-8 XML");
        assert!(patched.contains(r#"<row r="3">"#));
        assert!(patched.contains(r#"<rowBreaks count="1">"#));
    }
}
