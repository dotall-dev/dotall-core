use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;

#[derive(Debug, Clone)]
pub(super) struct FontPatch {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub name: Option<String>,
    pub size_pt: Option<f64>,
    pub color: Option<String>,
}

/// Append-only fonts/fills/cellXfs patch plus cell `s=` update.
pub(super) fn patch_font(
    styles_xml: &[u8],
    sheet_xml: &[u8],
    address: &str,
    font: &FontPatch,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let styles = std::str::from_utf8(styles_xml)
        .map_err(|error| writer_error(format!("styles XML is not UTF-8: {error}")))?;
    let sheet = std::str::from_utf8(sheet_xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;

    let style_index = cell_style_index(sheet, address)?.unwrap_or(0);
    let base_xf = cell_xf_at(styles, style_index)?;
    let font_xml = render_font(font);
    let (styles_out, new_font_id) = append_font(styles, &font_xml)?;
    let new_xf = render_xf_with_font(&base_xf, new_font_id);
    let (styles_out, new_xf_id) = append_cell_xf(&styles_out, &new_xf)?;
    let sheet_out = set_cell_style(sheet, address, new_xf_id)?;
    Ok((styles_out.into_bytes(), sheet_out.into_bytes()))
}

/// Append-only fills/cellXfs patch (or fillId 0 clear) plus cell `s=` update.
pub(super) fn patch_fill(
    styles_xml: &[u8],
    sheet_xml: &[u8],
    address: &str,
    color: Option<&str>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let styles = std::str::from_utf8(styles_xml)
        .map_err(|error| writer_error(format!("styles XML is not UTF-8: {error}")))?;
    let sheet = std::str::from_utf8(sheet_xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;

    let style_index = cell_style_index(sheet, address)?.unwrap_or(0);
    let base_xf = cell_xf_at(styles, style_index)?;
    let (styles_out, fill_id) = match color {
        Some(rgb) => {
            let fill_xml = format!(
                r#"<fill><patternFill patternType="solid"><fgColor rgb="{rgb}"/></patternFill></fill>"#
            );
            append_fill(styles, &fill_xml)?
        }
        None => (styles.to_owned(), 0),
    };
    let new_xf = render_xf_with_fill(&base_xf, fill_id);
    let (styles_out, new_xf_id) = append_cell_xf(&styles_out, &new_xf)?;
    let sheet_out = set_cell_style(sheet, address, new_xf_id)?;
    Ok((styles_out.into_bytes(), sheet_out.into_bytes()))
}

fn render_font(font: &FontPatch) -> String {
    let mut parts = String::from("<font>");
    if font.bold == Some(true) {
        parts.push_str("<b/>");
    } else if font.bold == Some(false) {
        parts.push_str(r#"<b val="0"/>"#);
    }
    if font.italic == Some(true) {
        parts.push_str("<i/>");
    } else if font.italic == Some(false) {
        parts.push_str(r#"<i val="0"/>"#);
    }
    if let Some(size) = font.size_pt {
        parts.push_str(&format!(r#"<sz val="{size}"/>"#));
    }
    if let Some(color) = &font.color {
        parts.push_str(&format!(r#"<color rgb="{color}"/>"#));
    }
    if let Some(name) = &font.name {
        let escaped = escape_xml_attr(name);
        parts.push_str(&format!(r#"<name val="{escaped}"/>"#));
    }
    parts.push_str("</font>");
    parts
}

fn render_xf_with_font(base: &XfAttrs, font_id: u32) -> String {
    format!(
        r#"<xf numFmtId="{}" fontId="{font_id}" fillId="{}" borderId="{}" xfId="{}"{} applyFont="1"/>"#,
        base.num_fmt_id,
        base.fill_id,
        base.border_id,
        base.xf_id,
        base.other_attrs_except(&["fontId", "applyFont"])
    )
}

fn render_xf_with_fill(base: &XfAttrs, fill_id: u32) -> String {
    format!(
        r#"<xf numFmtId="{}" fontId="{}" fillId="{fill_id}" borderId="{}" xfId="{}"{} applyFill="1"/>"#,
        base.num_fmt_id,
        base.font_id,
        base.border_id,
        base.xf_id,
        base.other_attrs_except(&["fillId", "applyFill"])
    )
}

#[derive(Debug, Clone)]
struct XfAttrs {
    num_fmt_id: String,
    font_id: String,
    fill_id: String,
    border_id: String,
    xf_id: String,
    /// Remaining attributes as ` key="value"` fragments (leading space included per attr).
    other: Vec<(String, String)>,
}

impl XfAttrs {
    fn other_attrs_except(&self, exclude: &[&str]) -> String {
        self.other
            .iter()
            .filter(|(key, _)| !exclude.iter().any(|ex| ex.eq_ignore_ascii_case(key)))
            .map(|(key, value)| format!(r#" {key}="{value}""#))
            .collect()
    }
}

fn cell_xf_at(styles: &str, index: u32) -> Result<XfAttrs> {
    let (inner, _) = section_inner(styles, "cellXfs")?;
    let xf = nth_child_element(inner, "xf", index as usize)?
        .ok_or_else(|| writer_error(format!("cellXfs index {index} not found")))?;
    parse_xf_attrs(xf)
}

fn parse_xf_attrs(open_tag: &str) -> Result<XfAttrs> {
    let attrs = parse_attributes(open_tag)?;
    let get = |attrs: &[(String, String)], key: &str| -> String {
        attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| "0".into())
    };
    let reserved = ["numFmtId", "fontId", "fillId", "borderId", "xfId"];
    let other = attrs
        .iter()
        .filter(|(k, _)| !reserved.iter().any(|r| r.eq_ignore_ascii_case(k)))
        .cloned()
        .collect();
    Ok(XfAttrs {
        num_fmt_id: get(&attrs, "numFmtId"),
        font_id: get(&attrs, "fontId"),
        fill_id: get(&attrs, "fillId"),
        border_id: get(&attrs, "borderId"),
        xf_id: get(&attrs, "xfId"),
        other,
    })
}

fn append_font(styles: &str, font_xml: &str) -> Result<(String, u32)> {
    append_child_in_section(styles, "fonts", "font", font_xml)
}

fn append_fill(styles: &str, fill_xml: &str) -> Result<(String, u32)> {
    append_child_in_section(styles, "fills", "fill", fill_xml)
}

fn append_cell_xf(styles: &str, xf_xml: &str) -> Result<(String, u32)> {
    append_child_in_section(styles, "cellXfs", "xf", xf_xml)
}

fn append_child_in_section(
    styles: &str,
    section: &str,
    child: &str,
    child_xml: &str,
) -> Result<(String, u32)> {
    let (open_start, open_end, close_start, _close_end) = find_section(styles, section)?
        .ok_or_else(|| writer_error(format!("styles.xml is missing <{section}>")))?;
    let open_tag = &styles[open_start..open_end];
    let inner = &styles[open_end..close_start];
    let count = count_top_level_children(inner, child)?;
    let new_count = count + 1;
    let new_open = set_count_attr(open_tag, new_count)?;
    let mut rebuilt = String::with_capacity(styles.len() + child_xml.len() + 16);
    rebuilt.push_str(&styles[..open_start]);
    rebuilt.push_str(&new_open);
    rebuilt.push_str(inner);
    rebuilt.push_str(child_xml);
    rebuilt.push_str(&styles[close_start..]);
    Ok((rebuilt, count))
}

fn section_inner<'a>(styles: &'a str, section: &str) -> Result<(&'a str, u32)> {
    let (open_start, open_end, close_start, _) = find_section(styles, section)?
        .ok_or_else(|| writer_error(format!("styles.xml is missing <{section}>")))?;
    let open_tag = &styles[open_start..open_end];
    let count = attr_u32(open_tag, "count")?.unwrap_or(0);
    Ok((&styles[open_end..close_start], count))
}

fn find_section(styles: &str, name: &str) -> Result<Option<(usize, usize, usize, usize)>> {
    let Some(rel) = find_named_open(styles, 0, name) else {
        return Ok(None);
    };
    let open_end = styles[rel..]
        .find('>')
        .map(|offset| rel + offset + 1)
        .ok_or_else(|| writer_error(format!("unterminated <{name}>")))?;
    if styles[rel..open_end].ends_with("/>") {
        return Err(writer_error(format!(
            "<{name}/> is empty; expected a container with children"
        )));
    }
    let close = format!("</{name}>");
    let close_rel = styles[open_end..]
        .find(&close)
        .ok_or_else(|| writer_error(format!("<{name}> is missing a closing tag")))?;
    let close_start = open_end + close_rel;
    let close_end = close_start + close.len();
    Ok(Some((rel, open_end, close_start, close_end)))
}

fn count_top_level_children(inner: &str, local: &str) -> Result<u32> {
    let mut count = 0_u32;
    let mut from = 0;
    while let Some(start) = find_named_open(inner, from, local) {
        let open_end = inner[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error(format!("unterminated <{local}>")))?;
        if inner[start..open_end].ends_with("/>") {
            from = open_end;
        } else {
            let close = format!("</{local}>");
            let close_rel = inner[open_end..]
                .find(&close)
                .ok_or_else(|| writer_error(format!("<{local}> is missing a closing tag")))?;
            from = open_end + close_rel + close.len();
        }
        count += 1;
    }
    Ok(count)
}

fn nth_child_element<'a>(inner: &'a str, local: &str, index: usize) -> Result<Option<&'a str>> {
    let mut from = 0;
    let mut seen = 0_usize;
    while let Some(start) = find_named_open(inner, from, local) {
        let open_end = inner[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error(format!("unterminated <{local}>")))?;
        let end = if inner[start..open_end].ends_with("/>") {
            open_end
        } else {
            let close = format!("</{local}>");
            let close_rel = inner[open_end..]
                .find(&close)
                .ok_or_else(|| writer_error(format!("<{local}> is missing a closing tag")))?;
            open_end + close_rel + close.len()
        };
        if seen == index {
            return Ok(Some(&inner[start..open_end]));
        }
        seen += 1;
        from = end;
    }
    Ok(None)
}

fn set_count_attr(open_tag: &str, count: u32) -> Result<String> {
    let trimmed = open_tag.trim_end_matches('>').trim_end_matches('/');
    let self_closing = open_tag.trim_end().ends_with("/>");
    if let Some(existing) = attr_span(trimmed, "count")? {
        let mut rebuilt = String::with_capacity(open_tag.len() + 8);
        rebuilt.push_str(&trimmed[..existing.0]);
        rebuilt.push_str(&format!(r#"count="{count}""#));
        rebuilt.push_str(&trimmed[existing.1..]);
        if self_closing {
            rebuilt.push_str("/>");
        } else {
            rebuilt.push('>');
        }
        return Ok(rebuilt);
    }
    let mut rebuilt = String::with_capacity(open_tag.len() + 16);
    rebuilt.push_str(trimmed);
    rebuilt.push_str(&format!(r#" count="{count}""#));
    if self_closing {
        rebuilt.push_str("/>");
    } else {
        rebuilt.push('>');
    }
    Ok(rebuilt)
}

fn cell_style_index(sheet: &str, address: &str) -> Result<Option<u32>> {
    let Some((start, end)) = find_cell(sheet, address)? else {
        return Ok(None);
    };
    let open = &sheet[start..end];
    attr_u32(open, "s")
}

fn set_cell_style(sheet: &str, address: &str, style_index: u32) -> Result<String> {
    if let Some((start, end)) = find_cell(sheet, address)? {
        let open = &sheet[start..end];
        let new_open = set_or_replace_attr(open, "s", &style_index.to_string())?;
        let mut rebuilt = String::with_capacity(sheet.len() + 8);
        rebuilt.push_str(&sheet[..start]);
        rebuilt.push_str(&new_open);
        rebuilt.push_str(&sheet[end..]);
        return Ok(rebuilt);
    }
    insert_style_only_cell(sheet, address, style_index)
}

fn insert_style_only_cell(sheet: &str, address: &str, style_index: u32) -> Result<String> {
    let row = row_number(address)?;
    let cell = format!(r#"<c r="{address}" s="{style_index}"/>"#);
    if let Some(row_end) = find_row_close(sheet, row)? {
        let mut rebuilt = String::with_capacity(sheet.len() + cell.len());
        rebuilt.push_str(&sheet[..row_end]);
        rebuilt.push_str(&cell);
        rebuilt.push_str(&sheet[row_end..]);
        return Ok(rebuilt);
    }
    let row_xml = format!(r#"<row r="{row}">{cell}</row>"#);
    let insertion = sheet
        .find("</sheetData>")
        .ok_or_else(|| writer_error("worksheet is missing sheetData"))?;
    let mut rebuilt = String::with_capacity(sheet.len() + row_xml.len());
    rebuilt.push_str(&sheet[..insertion]);
    rebuilt.push_str(&row_xml);
    rebuilt.push_str(&sheet[insertion..]);
    Ok(rebuilt)
}

fn find_cell(sheet: &str, address: &str) -> Result<Option<(usize, usize)>> {
    let patterns = [
        format!(r#"<c r="{address}""#),
        format!(r#"<c r="{address}">"#),
        format!(r#"<c r="{address}"/>"#),
    ];
    let mut best = None;
    for pattern in &patterns {
        if let Some(rel) = sheet.find(pattern)
            && best.is_none_or(|current| rel < current)
        {
            best = Some(rel);
        }
    }
    let Some(start) = best else {
        return Ok(None);
    };
    let open_end = sheet[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| writer_error(format!("unterminated cell {address}")))?;
    Ok(Some((start, open_end)))
}

fn find_row_close(sheet: &str, row: u32) -> Result<Option<usize>> {
    let patterns = [format!(r#"<row r="{row}""#), format!(r#"<row r="{row}">"#)];
    let mut start = None;
    for pattern in &patterns {
        if let Some(rel) = sheet.find(pattern)
            && start.is_none_or(|current| rel < current)
        {
            start = Some(rel);
        }
    }
    let Some(start) = start else {
        return Ok(None);
    };
    let open_end = sheet[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| writer_error(format!("unterminated row {row}")))?;
    if sheet[start..open_end].ends_with("/>") {
        return Ok(None);
    }
    let close = "</row>";
    let close_rel = sheet[open_end..]
        .find(close)
        .ok_or_else(|| writer_error(format!("row {row} is missing a closing tag")))?;
    Ok(Some(open_end + close_rel))
}

fn row_number(address: &str) -> Result<u32> {
    let digits = address
        .bytes()
        .skip_while(|b| b.is_ascii_alphabetic())
        .collect::<Vec<_>>();
    std::str::from_utf8(&digits)
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|row| *row > 0)
        .ok_or_else(|| writer_error(format!("invalid cell address `{address}`")))
}

fn set_or_replace_attr(open_tag: &str, key: &str, value: &str) -> Result<String> {
    let self_closing = open_tag.trim_end().ends_with("/>");
    let trimmed = open_tag.trim_end_matches('>').trim_end_matches('/');
    if let Some((attr_start, attr_end)) = attr_span(trimmed, key)? {
        let mut rebuilt = String::with_capacity(open_tag.len() + value.len());
        rebuilt.push_str(&trimmed[..attr_start]);
        rebuilt.push_str(&format!(r#"{key}="{value}""#));
        rebuilt.push_str(&trimmed[attr_end..]);
        if self_closing {
            rebuilt.push_str("/>");
        } else {
            rebuilt.push('>');
        }
        return Ok(rebuilt);
    }
    let mut rebuilt = String::with_capacity(open_tag.len() + key.len() + value.len() + 4);
    rebuilt.push_str(trimmed);
    rebuilt.push_str(&format!(r#" {key}="{value}""#));
    if self_closing {
        rebuilt.push_str("/>");
    } else {
        rebuilt.push('>');
    }
    Ok(rebuilt)
}

fn attr_u32(open_tag: &str, key: &str) -> Result<Option<u32>> {
    let Some((_, value)) = parse_attributes(open_tag)?
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
    else {
        return Ok(None);
    };
    value
        .parse()
        .map(Some)
        .map_err(|_| writer_error(format!("invalid `{key}` attribute value `{value}`")))
}

fn attr_span(open_tag: &str, key: &str) -> Result<Option<(usize, usize)>> {
    let needle = format!("{key}=");
    let mut from = 0;
    while let Some(rel) = open_tag[from..].find(&needle) {
        let abs = from + rel;
        let before_ok = abs == 0
            || open_tag
                .as_bytes()
                .get(abs - 1)
                .is_some_and(|b| b.is_ascii_whitespace());
        if !before_ok {
            from = abs + 1;
            continue;
        }
        let value_start = abs + needle.len();
        let bytes = open_tag.as_bytes();
        if value_start >= bytes.len() || bytes[value_start] != b'"' {
            return Err(writer_error(format!("malformed `{key}` attribute")));
        }
        let close = open_tag[value_start + 1..]
            .find('"')
            .map(|offset| value_start + 1 + offset)
            .ok_or_else(|| writer_error(format!("unterminated `{key}` attribute")))?;
        return Ok(Some((abs, close + 1)));
    }
    Ok(None)
}

fn parse_attributes(open_tag: &str) -> Result<Vec<(String, String)>> {
    let mut attrs = Vec::new();
    let bytes = open_tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] != b' ' && bytes[i] != b'/' && bytes[i] != b'>' {
        i += 1;
    }
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'/' || bytes[i] == b'>' {
            break;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = std::str::from_utf8(&bytes[key_start..i])
            .map_err(|error| writer_error(format!("invalid attribute name: {error}")))?
            .to_owned();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            return Err(writer_error(format!("malformed attribute `{key}`")));
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'"' {
            return Err(writer_error(format!(
                "attribute `{key}` is missing a quoted value"
            )));
        }
        i += 1;
        let value_start = i;
        while i < bytes.len() && bytes[i] != b'"' {
            i += 1;
        }
        if i >= bytes.len() {
            return Err(writer_error(format!("unterminated attribute `{key}`")));
        }
        let value = std::str::from_utf8(&bytes[value_start..i])
            .map_err(|error| writer_error(format!("invalid attribute value: {error}")))?
            .to_owned();
        i += 1;
        attrs.push((key, value));
    }
    Ok(attrs)
}

fn find_named_open(xml: &str, from: usize, local: &str) -> Option<usize> {
    let patterns = [
        format!("<{local} "),
        format!("<{local}>"),
        format!("<{local}/>"),
    ];
    let mut best = None;
    for pattern in &patterns {
        if let Some(rel) = xml[from..].find(pattern) {
            let abs = from + rel;
            if best.is_none_or(|current| abs < current) {
                best = Some(abs);
            }
        }
    }
    best
}

fn escape_xml_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn writer_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx cell_style writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_font_and_xf_updates_counts() {
        let styles = r#"<?xml version="1.0"?><styleSheet><fonts count="1"><font><sz val="11"/></font></fonts><fills count="1"><fill><patternFill patternType="none"/></fill></fills><cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs></styleSheet>"#;
        let sheet = r#"<?xml version="1.0"?><worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#;
        let (styles_out, sheet_out) = patch_font(
            styles.as_bytes(),
            sheet.as_bytes(),
            "A1",
            &FontPatch {
                bold: Some(true),
                italic: None,
                name: Some("Calibri".into()),
                size_pt: Some(14.0),
                color: Some("FF1F4E79".into()),
            },
        )
        .expect("patch");
        let styles_text = String::from_utf8(styles_out).expect("utf8");
        let sheet_text = String::from_utf8(sheet_out).expect("utf8");
        assert!(styles_text.contains(r#"fonts count="2""#));
        assert!(styles_text.contains(r#"cellXfs count="2""#));
        assert!(styles_text.contains("<b/>"));
        assert!(styles_text.contains("FF1F4E79"));
        assert!(sheet_text.contains(r#"s="1""#));
    }

    #[test]
    fn clear_fill_uses_fill_id_zero() {
        let styles = r#"<?xml version="1.0"?><styleSheet><fonts count="1"><font><sz val="11"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs></styleSheet>"#;
        let sheet = r#"<?xml version="1.0"?><worksheet><sheetData><row r="1"><c r="A1" s="0"><v>1</v></c></row></sheetData></worksheet>"#;
        let (styles_out, _) =
            patch_fill(styles.as_bytes(), sheet.as_bytes(), "A1", None).expect("patch");
        let styles_text = String::from_utf8(styles_out).expect("utf8");
        assert!(styles_text.contains(r#"fillId="0""#));
        assert!(styles_text.contains(r#"applyFill="1""#));
        assert!(styles_text.contains(r#"fills count="2""#));
    }
}
