use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;
use crate::edits::transform::{MAX_COLUMNS, column_number};

pub(super) enum DimensionEdit {
    ColumnWidth { column: String, width: f64 },
    RowHeight { row: u32, height: f64 },
}

pub(super) fn patch(xml: &[u8], edit: &DimensionEdit) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    match edit {
        DimensionEdit::ColumnWidth { column, width } => set_column_width(source, column, *width),
        DimensionEdit::RowHeight { row, height } => set_row_height(source, *row, *height),
    }
}

fn set_column_width(source: &str, column: &str, width: f64) -> Result<Vec<u8>> {
    let index = column_number(column);
    if index == 0 || index > MAX_COLUMNS {
        return Err(writer_error(format!("invalid column `{column}`")));
    }
    let width_text = format_dimension(width);
    let mut cols = parse_cols(source)?;
    apply_column_width(&mut cols, index, &width_text);
    let rendered = render_cols(&cols);
    replace_or_insert_cols(source, &rendered)
}

fn set_row_height(source: &str, row: u32, height: f64) -> Result<Vec<u8>> {
    let height_text = format_dimension(height);
    if let Some((start, end)) = find_row_open_tag(source, row)? {
        let with_ht = set_or_replace_attr(&source[start..end], "ht", &height_text)?;
        let updated = set_or_replace_attr(&with_ht, "customHeight", "1")?;
        let mut output = String::with_capacity(source.len() + updated.len());
        output.push_str(&source[..start]);
        output.push_str(&updated);
        output.push_str(&source[end..]);
        return Ok(output.into_bytes());
    }

    let insertion = row_insertion_point(source, row)?;
    let row_tag = format!(r#"<row r="{row}" ht="{height_text}" customHeight="1"/>"#);
    let mut output = String::with_capacity(source.len() + row_tag.len());
    output.push_str(&source[..insertion]);
    output.push_str(&row_tag);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

#[derive(Debug, Clone)]
struct ColEntry {
    min: u32,
    max: u32,
    width: Option<String>,
    custom_width: bool,
    extras: Vec<(String, String)>,
}

fn apply_column_width(cols: &mut Vec<ColEntry>, index: u32, width: &str) {
    let mut next = Vec::with_capacity(cols.len() + 2);
    let mut replaced = false;
    for entry in cols.drain(..) {
        if entry.max < index || entry.min > index {
            next.push(entry);
            continue;
        }
        replaced = true;
        if entry.min < index {
            next.push(ColEntry {
                min: entry.min,
                max: index - 1,
                width: entry.width.clone(),
                custom_width: entry.custom_width,
                extras: entry.extras.clone(),
            });
        }
        next.push(ColEntry {
            min: index,
            max: index,
            width: Some(width.to_owned()),
            custom_width: true,
            extras: entry.extras.clone(),
        });
        if entry.max > index {
            next.push(ColEntry {
                min: index + 1,
                max: entry.max,
                width: entry.width,
                custom_width: entry.custom_width,
                extras: entry.extras,
            });
        }
    }
    if !replaced {
        next.push(ColEntry {
            min: index,
            max: index,
            width: Some(width.to_owned()),
            custom_width: true,
            extras: Vec::new(),
        });
    }
    next.sort_by_key(|entry| entry.min);
    *cols = next;
}

fn parse_cols(source: &str) -> Result<Vec<ColEntry>> {
    let Some((start, _end)) = find_cols_block(source)? else {
        return Ok(Vec::new());
    };
    let open_end = source[start..]
        .find('>')
        .map(|offset| start + offset)
        .ok_or_else(|| writer_error("unterminated cols"))?;
    let open_tag = &source[start..=open_end];
    if open_tag.ends_with("/>") {
        return Ok(Vec::new());
    }
    let close = source[open_end + 1..]
        .find("</cols>")
        .or_else(|| source[open_end + 1..].find("</x:cols>"))
        .map(|offset| open_end + 1 + offset)
        .ok_or_else(|| writer_error("unterminated cols element"))?;
    let body = &source[open_end + 1..close];
    let mut cols = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = find_open_tag(&body[cursor..], "col") {
        let tag_start = cursor + relative;
        let tag_end = body[tag_start..]
            .find('>')
            .map(|offset| tag_start + offset)
            .ok_or_else(|| writer_error("unterminated col"))?;
        let tag = &body[tag_start..=tag_end];
        cols.push(parse_col_tag(tag)?);
        cursor = tag_end + 1;
    }
    Ok(cols)
}

fn parse_col_tag(tag: &str) -> Result<ColEntry> {
    let min = attribute(tag, "min")
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| writer_error("col is missing required `min` attribute"))?;
    let max = attribute(tag, "max")
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| writer_error("col is missing required `max` attribute"))?;
    if min == 0 || max == 0 || min > max {
        return Err(writer_error(format!(
            "col has invalid min/max range `{min}`..`{max}`"
        )));
    }
    let width = attribute(tag, "width").map(str::to_owned);
    let custom_width = attribute(tag, "customWidth")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let mut extras = Vec::new();
    let mut search = 0;
    while let Some(relative) = tag[search..].find('=') {
        let eq = search + relative;
        let key_start = tag[..eq]
            .rfind([' ', '\t', '\n', '\r', '<'])
            .map(|index| index + 1)
            .unwrap_or(0);
        let key = tag[key_start..eq].trim();
        if key.is_empty() || matches!(key, "min" | "max" | "width" | "customWidth") {
            search = eq + 1;
            continue;
        }
        let value_start = eq + 2;
        if !tag[eq..].starts_with("=\"") {
            search = eq + 1;
            continue;
        }
        let value_end = tag[value_start..]
            .find('"')
            .map(|offset| value_start + offset)
            .ok_or_else(|| writer_error("unterminated col attribute"))?;
        extras.push((key.to_owned(), tag[value_start..value_end].to_owned()));
        search = value_end + 1;
    }
    Ok(ColEntry {
        min,
        max,
        width,
        custom_width,
        extras,
    })
}

fn render_cols(cols: &[ColEntry]) -> String {
    if cols.is_empty() {
        return String::new();
    }
    let mut output = String::from("<cols>");
    for entry in cols {
        output.push_str("<col");
        output.push_str(&format!(r#" min="{}" max="{}""#, entry.min, entry.max));
        if let Some(width) = &entry.width {
            output.push_str(&format!(r#" width="{width}""#));
        }
        if entry.custom_width {
            output.push_str(r#" customWidth="1""#);
        }
        for (key, value) in &entry.extras {
            output.push_str(&format!(r#" {key}="{value}""#));
        }
        output.push_str("/>");
    }
    output.push_str("</cols>");
    output
}

fn replace_or_insert_cols(source: &str, rendered: &str) -> Result<Vec<u8>> {
    if let Some((start, end)) = find_cols_block(source)? {
        let mut output = String::with_capacity(source.len() + rendered.len());
        output.push_str(&source[..start]);
        output.push_str(rendered);
        output.push_str(&source[end..]);
        return Ok(output.into_bytes());
    }
    if rendered.is_empty() {
        return Ok(source.as_bytes().to_vec());
    }
    let insertion = source
        .find("<sheetData")
        .or_else(|| source.find("<x:sheetData"))
        .ok_or_else(|| writer_error("worksheet XML is missing sheetData"))?;
    let mut output = String::with_capacity(source.len() + rendered.len());
    output.push_str(&source[..insertion]);
    output.push_str(rendered);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn find_cols_block(source: &str) -> Result<Option<(usize, usize)>> {
    let Some(start) = find_open_tag(source, "cols") else {
        return Ok(None);
    };
    let open_end = source[start..]
        .find('>')
        .map(|offset| start + offset)
        .ok_or_else(|| writer_error("unterminated cols"))?;
    let open_tag = &source[start..=open_end];
    if open_tag.ends_with("/>") {
        return Ok(Some((start, open_end + 1)));
    }
    let close = source[open_end + 1..]
        .find("</cols>")
        .map(|offset| open_end + 1 + offset + "</cols>".len())
        .or_else(|| {
            source[open_end + 1..]
                .find("</x:cols>")
                .map(|offset| open_end + 1 + offset + "</x:cols>".len())
        })
        .ok_or_else(|| writer_error("unterminated cols element"))?;
    Ok(Some((start, close)))
}

fn find_row_open_tag(source: &str, row: u32) -> Result<Option<(usize, usize)>> {
    let mut cursor = 0;
    while let Some(relative) = find_open_tag(&source[cursor..], "row") {
        let start = cursor + relative;
        let end = source[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error("unterminated row"))?;
        let tag = &source[start..end];
        if attribute(tag, "r").and_then(|value| value.parse::<u32>().ok()) == Some(row) {
            return Ok(Some((start, end)));
        }
        cursor = end;
    }
    Ok(None)
}

fn row_insertion_point(source: &str, row: u32) -> Result<usize> {
    let sheet_data_start = source
        .find("<sheetData")
        .or_else(|| source.find("<x:sheetData"))
        .ok_or_else(|| writer_error("worksheet XML is missing sheetData"))?;
    let open_end = source[sheet_data_start..]
        .find('>')
        .map(|offset| sheet_data_start + offset + 1)
        .ok_or_else(|| writer_error("unterminated sheetData"))?;
    if source[sheet_data_start..open_end].contains("/>") {
        return Err(writer_error(
            "cannot insert row height into empty self-closing sheetData",
        ));
    }
    let mut cursor = open_end;
    while let Some(relative) = find_open_tag(&source[cursor..], "row") {
        let start = cursor + relative;
        let end = source[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error("unterminated row"))?;
        let existing = attribute(&source[start..end], "r")
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| writer_error("row is missing required `r` attribute"))?;
        if existing > row {
            return Ok(start);
        }
        cursor = end;
    }
    source[open_end..]
        .find("</sheetData>")
        .or_else(|| source[open_end..].find("</x:sheetData>"))
        .map(|offset| open_end + offset)
        .ok_or_else(|| writer_error("unterminated sheetData element"))
}

fn set_or_replace_attr(tag: &str, name: &str, value: &str) -> Result<String> {
    let needle = format!("{name}=\"");
    if let Some(attr_start) = tag.find(&needle) {
        let value_start = attr_start + needle.len();
        let value_end = tag[value_start..]
            .find('"')
            .map(|offset| value_start + offset)
            .ok_or_else(|| writer_error(format!("unterminated `{name}` attribute")))?;
        let mut output = String::with_capacity(tag.len() + value.len());
        output.push_str(&tag[..value_start]);
        output.push_str(value);
        output.push_str(&tag[value_end..]);
        return Ok(output);
    }
    let insert_at = if tag.ends_with("/>") {
        tag.len() - 2
    } else if tag.ends_with('>') {
        tag.len() - 1
    } else {
        return Err(writer_error("row open tag is missing '>'"));
    };
    let mut output = String::with_capacity(tag.len() + name.len() + value.len() + 4);
    output.push_str(&tag[..insert_at]);
    output.push_str(&format!(r#" {name}="{value}""#));
    output.push_str(&tag[insert_at..]);
    Ok(output)
}

fn format_dimension(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() <= i64::MAX as f64 {
        format!("{}", value as i64)
    } else {
        let formatted = format!("{value}");
        formatted
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn find_open_tag(source: &str, local: &str) -> Option<usize> {
    let patterns = [format!("<{local}"), format!("<x:{local}")];
    let mut best = None;
    for pattern in &patterns {
        let mut search_from = 0;
        while let Some(relative) = source[search_from..].find(pattern.as_str()) {
            let index = search_from + relative;
            let after = index + pattern.len();
            if matches!(
                source.as_bytes().get(after),
                Some(b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/')
            ) {
                best = Some(match best {
                    Some(current) if current < index => current,
                    _ => index,
                });
                break;
            }
            search_from = after;
        }
    }
    best
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

fn writer_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx dimension writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_cols_before_sheet_data() {
        let xml = br#"<worksheet><sheetData><row r="1"/></sheetData></worksheet>"#;
        let patched = patch(
            xml,
            &DimensionEdit::ColumnWidth {
                column: "A".into(),
                width: 22.5,
            },
        )
        .expect("set width");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(
            r#"<cols><col min="1" max="1" width="22.5" customWidth="1"/></cols><sheetData"#
        ));
    }

    #[test]
    fn updates_existing_row_height() {
        let xml = br#"<worksheet><sheetData><row r="1" spans="1:2"><c r="A1"/></row></sheetData></worksheet>"#;
        let patched = patch(
            xml,
            &DimensionEdit::RowHeight {
                row: 1,
                height: 36.0,
            },
        )
        .expect("set height");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"<row r="1" spans="1:2" ht="36" customHeight="1">"#));
    }

    #[test]
    fn splits_multi_column_col_entry() {
        let xml = br#"<worksheet><cols><col min="1" max="3" width="10" customWidth="1"/></cols><sheetData/></worksheet>"#;
        let patched = patch(
            xml,
            &DimensionEdit::ColumnWidth {
                column: "B".into(),
                width: 20.0,
            },
        )
        .expect("split");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"min="1" max="1" width="10""#));
        assert!(text.contains(r#"min="2" max="2" width="20""#));
        assert!(text.contains(r#"min="3" max="3" width="10""#));
    }
}
