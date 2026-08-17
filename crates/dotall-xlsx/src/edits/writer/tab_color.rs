use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;

/// Set or clear worksheet `sheetPr`/`tabColor` (`rgb` AARRGGBB). `None` clears.
pub(super) fn patch(xml: &[u8], color: Option<&str>) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    apply_tab_color(source, color)
}

fn apply_tab_color(source: &str, color: Option<&str>) -> Result<Vec<u8>> {
    if let Some(rgb) = color {
        return upsert_tab_color(source, rgb);
    }
    clear_tab_color(source)
}

fn upsert_tab_color(source: &str, rgb: &str) -> Result<Vec<u8>> {
    let tab_color = format!(r#"<tabColor rgb="{rgb}"/>"#);
    if let Some((sheet_pr_start, open_end, close_start, sheet_pr_end)) = find_sheet_pr(source)? {
        let open_tag = &source[sheet_pr_start..open_end];
        let mut rebuilt = String::with_capacity(source.len() + tab_color.len() + 32);
        rebuilt.push_str(&source[..sheet_pr_start]);
        if open_tag.ends_with("/>") {
            let open = open_tag.trim_end_matches("/>").trim_end();
            rebuilt.push_str(open);
            rebuilt.push('>');
            rebuilt.push_str(&tab_color);
            rebuilt.push_str("</sheetPr>");
        } else {
            let inner = &source[open_end..close_start];
            rebuilt.push_str(open_tag);
            if let Some((tab_start, tab_end)) = find_tab_color(inner)? {
                rebuilt.push_str(&inner[..tab_start]);
                rebuilt.push_str(&tab_color);
                rebuilt.push_str(&inner[tab_end..]);
            } else {
                rebuilt.push_str(&tab_color);
                rebuilt.push_str(inner);
            }
            rebuilt.push_str(&source[close_start..sheet_pr_end]);
        }
        rebuilt.push_str(&source[sheet_pr_end..]);
        return Ok(rebuilt.into_bytes());
    }

    let sheet_pr = format!("<sheetPr>{tab_color}</sheetPr>");
    let insertion = sheet_pr_insertion_point(source)?;
    let mut output = String::with_capacity(source.len() + sheet_pr.len());
    output.push_str(&source[..insertion]);
    output.push_str(&sheet_pr);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn clear_tab_color(source: &str) -> Result<Vec<u8>> {
    let Some((sheet_pr_start, open_end, close_start, sheet_pr_end)) = find_sheet_pr(source)? else {
        return Ok(source.as_bytes().to_vec());
    };
    let open_tag = &source[sheet_pr_start..open_end];
    if open_tag.ends_with("/>") {
        // Empty self-closing sheetPr with no tabColor — leave as-is.
        return Ok(source.as_bytes().to_vec());
    }
    let inner = &source[open_end..close_start];
    let Some((tab_start, tab_end)) = find_tab_color(inner)? else {
        return Ok(source.as_bytes().to_vec());
    };
    let remainder = format!("{}{}", &inner[..tab_start], &inner[tab_end..]);
    let mut rebuilt = String::with_capacity(source.len());
    rebuilt.push_str(&source[..sheet_pr_start]);
    if remainder.trim().is_empty() {
        // Drop empty sheetPr entirely.
    } else {
        rebuilt.push_str(open_tag);
        rebuilt.push_str(&remainder);
        rebuilt.push_str(&source[close_start..sheet_pr_end]);
    }
    rebuilt.push_str(&source[sheet_pr_end..]);
    Ok(rebuilt.into_bytes())
}

fn find_sheet_pr(source: &str) -> Result<Option<(usize, usize, usize, usize)>> {
    let Some(rel) = find_named_open(source, 0, "sheetPr") else {
        return Ok(None);
    };
    let open_end = source[rel..]
        .find('>')
        .map(|offset| rel + offset + 1)
        .ok_or_else(|| writer_error("unterminated sheetPr"))?;
    if source[rel..open_end].ends_with("/>") {
        return Ok(Some((rel, open_end, open_end, open_end)));
    }
    let close = "</sheetPr>";
    let close_rel = source[open_end..]
        .find(close)
        .ok_or_else(|| writer_error("sheetPr is missing a closing tag"))?;
    let close_start = open_end + close_rel;
    let sheet_pr_end = close_start + close.len();
    Ok(Some((rel, open_end, close_start, sheet_pr_end)))
}

fn find_tab_color(inner: &str) -> Result<Option<(usize, usize)>> {
    let Some(rel) = find_named_open(inner, 0, "tabColor") else {
        return Ok(None);
    };
    let open_end = inner[rel..]
        .find('>')
        .map(|offset| rel + offset + 1)
        .ok_or_else(|| writer_error("unterminated tabColor"))?;
    if inner[rel..open_end].ends_with("/>") {
        return Ok(Some((rel, open_end)));
    }
    let close = "</tabColor>";
    let close_rel = inner[open_end..]
        .find(close)
        .ok_or_else(|| writer_error("tabColor is missing a closing tag"))?;
    Ok(Some((rel, open_end + close_rel + close.len())))
}

fn sheet_pr_insertion_point(source: &str) -> Result<usize> {
    let open = source
        .find("<worksheet")
        .ok_or_else(|| writer_error("worksheet root element not found"))?;
    let open_end = source[open..]
        .find('>')
        .map(|offset| open + offset + 1)
        .ok_or_else(|| writer_error("unterminated worksheet root"))?;
    Ok(open_end)
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

fn writer_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx tab_color writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_creates_sheet_pr_when_missing() {
        let xml = r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#;
        let patched = patch(xml.as_bytes(), Some("FF4472C4")).expect("upsert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"<sheetPr><tabColor rgb="FF4472C4"/></sheetPr>"#));
        assert!(text.contains("<sheetData/>"));
    }

    #[test]
    fn clear_removes_tab_color_and_empty_sheet_pr() {
        let xml = r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetPr><tabColor rgb="FFFF0000"/></sheetPr><sheetData/></worksheet>"#;
        let patched = patch(xml.as_bytes(), None).expect("clear");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(!text.contains("tabColor"));
        assert!(!text.contains("sheetPr"));
    }
}
