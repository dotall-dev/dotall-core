use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;

/// Set or clear worksheet fit-to-page via `pageSetUpPr` + `pageSetup` fit attrs.
/// `None` for both width/height clears; `Some` sets both values.
pub(super) fn patch(xml: &[u8], width: Option<u32>, height: Option<u32>) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    match (width, height) {
        (Some(w), Some(h)) => apply_fit(source, w, h),
        (None, None) => clear_fit(source),
        _ => Err(writer_error(
            "fit_to_page requires both width and height, or both null to clear",
        )),
    }
}

fn apply_fit(source: &str, width: u32, height: u32) -> Result<Vec<u8>> {
    let with_pr = upsert_fit_to_page_flag(source, true)?;
    let with_pr = std::str::from_utf8(&with_pr)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    upsert_page_setup_fit(with_pr, Some(width), Some(height))
}

fn clear_fit(source: &str) -> Result<Vec<u8>> {
    let with_pr = upsert_fit_to_page_flag(source, false)?;
    let with_pr = std::str::from_utf8(&with_pr)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    upsert_page_setup_fit(with_pr, None, None)
}

fn upsert_fit_to_page_flag(source: &str, enabled: bool) -> Result<Vec<u8>> {
    let page_setup_pr = if enabled {
        r#"<pageSetUpPr fitToPage="1"/>"#
    } else {
        r#"<pageSetUpPr fitToPage="0"/>"#
    };
    if let Some((sheet_pr_start, open_end, close_start, sheet_pr_end)) = find_sheet_pr(source)? {
        let open_tag = &source[sheet_pr_start..open_end];
        let mut rebuilt = String::with_capacity(source.len() + page_setup_pr.len() + 32);
        rebuilt.push_str(&source[..sheet_pr_start]);
        if open_tag.ends_with("/>") {
            if !enabled {
                // Leave empty self-closing sheetPr alone when clearing.
                return Ok(source.as_bytes().to_vec());
            }
            let open = open_tag.trim_end_matches("/>").trim_end();
            rebuilt.push_str(open);
            rebuilt.push('>');
            rebuilt.push_str(page_setup_pr);
            rebuilt.push_str("</sheetPr>");
        } else {
            let inner = &source[open_end..close_start];
            if let Some((pr_start, pr_end)) = find_page_setup_pr(inner)? {
                if enabled {
                    rebuilt.push_str(open_tag);
                    rebuilt.push_str(&inner[..pr_start]);
                    rebuilt.push_str(page_setup_pr);
                    rebuilt.push_str(&inner[pr_end..]);
                    rebuilt.push_str(&source[close_start..sheet_pr_end]);
                } else {
                    let remainder = format!("{}{}", &inner[..pr_start], &inner[pr_end..]);
                    if remainder.trim().is_empty() {
                        // Drop empty sheetPr.
                    } else {
                        rebuilt.push_str(open_tag);
                        rebuilt.push_str(&remainder);
                        rebuilt.push_str(&source[close_start..sheet_pr_end]);
                    }
                }
            } else if enabled {
                rebuilt.push_str(open_tag);
                rebuilt.push_str(page_setup_pr);
                rebuilt.push_str(inner);
                rebuilt.push_str(&source[close_start..sheet_pr_end]);
            } else {
                rebuilt.push_str(open_tag);
                rebuilt.push_str(inner);
                rebuilt.push_str(&source[close_start..sheet_pr_end]);
            }
        }
        rebuilt.push_str(&source[sheet_pr_end..]);
        return Ok(rebuilt.into_bytes());
    }

    if !enabled {
        return Ok(source.as_bytes().to_vec());
    }
    let sheet_pr = format!("<sheetPr>{page_setup_pr}</sheetPr>");
    let insertion = sheet_pr_insertion_point(source)?;
    let mut output = String::with_capacity(source.len() + sheet_pr.len());
    output.push_str(&source[..insertion]);
    output.push_str(&sheet_pr);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn upsert_page_setup_fit(source: &str, width: Option<u32>, height: Option<u32>) -> Result<Vec<u8>> {
    if let Some((start, end)) = find_page_setup(source)? {
        let existing = &source[start..end];
        let patched = match (width, height) {
            (Some(w), Some(h)) => upsert_fit_attrs(existing, w, h)?,
            (None, None) => clear_fit_attrs(existing)?,
            _ => {
                return Err(writer_error(
                    "fit_to_page requires both width and height, or both null to clear",
                ));
            }
        };
        let mut output = String::with_capacity(source.len() + patched.len());
        output.push_str(&source[..start]);
        output.push_str(&patched);
        output.push_str(&source[end..]);
        return Ok(output.into_bytes());
    }
    match (width, height) {
        (Some(w), Some(h)) => {
            let tag = format!(r#"<pageSetup fitToWidth="{w}" fitToHeight="{h}"/>"#);
            let insertion = insert_page_setup_at(source)
                .ok_or_else(|| writer_error("worksheet XML is missing closing worksheet tag"))?;
            let mut output = String::with_capacity(source.len() + tag.len());
            output.push_str(&source[..insertion]);
            output.push_str(&tag);
            output.push_str(&source[insertion..]);
            Ok(output.into_bytes())
        }
        (None, None) => Ok(source.as_bytes().to_vec()),
        _ => Err(writer_error(
            "fit_to_page requires both width and height, or both null to clear",
        )),
    }
}

fn upsert_fit_attrs(page_setup: &str, width: u32, height: u32) -> Result<String> {
    let open_end = page_setup
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| writer_error("unterminated pageSetup"))?;
    let open = &page_setup[..open_end];
    let rest = &page_setup[open_end..];
    let without = remove_attr(&remove_attr(open, "fitToWidth"), "fitToHeight");
    let self_closing = without.ends_with("/>");
    let base = if self_closing {
        without.trim_end_matches("/>").trim_end()
    } else {
        without.trim_end_matches('>').trim_end()
    };
    if self_closing {
        Ok(format!(
            r#"{base} fitToWidth="{width}" fitToHeight="{height}"/>"#
        ))
    } else {
        Ok(format!(
            r#"{base} fitToWidth="{width}" fitToHeight="{height}">{rest}"#
        ))
    }
}

fn clear_fit_attrs(page_setup: &str) -> Result<String> {
    let open_end = page_setup
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| writer_error("unterminated pageSetup"))?;
    let open = &page_setup[..open_end];
    let rest = &page_setup[open_end..];
    let without = remove_attr(&remove_attr(open, "fitToWidth"), "fitToHeight");
    Ok(format!("{without}{rest}"))
}

fn remove_attr(open: &str, name: &str) -> String {
    let needle = format!("{name}=\"");
    let Some(rel) = open.find(&needle) else {
        return open.to_owned();
    };
    let value_start = rel + needle.len();
    let Some(value_end_rel) = open[value_start..].find('"') else {
        return open.to_owned();
    };
    let end = value_start + value_end_rel + 1;
    let mut start = rel;
    while start > 0 && open.as_bytes()[start - 1] == b' ' {
        start -= 1;
    }
    format!("{}{}", &open[..start], &open[end..])
}

fn insert_page_setup_at(source: &str) -> Option<usize> {
    if let Some(offset) = source.find("<headerFooter") {
        return Some(offset);
    }
    if let Some(offset) = source.rfind("</worksheet>") {
        return Some(offset);
    }
    None
}

fn find_page_setup(source: &str) -> Result<Option<(usize, usize)>> {
    let Some(start) = find_open_tag(source, "pageSetup") else {
        return Ok(None);
    };
    let open_end = source[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| writer_error("unterminated pageSetup"))?;
    if source[start..open_end].ends_with("/>") {
        return Ok(Some((start, open_end)));
    }
    let close = "</pageSetup>";
    let end = source[open_end..]
        .find(close)
        .map(|offset| open_end + offset + close.len())
        .ok_or_else(|| writer_error("missing closing </pageSetup>"))?;
    Ok(Some((start, end)))
}

fn find_sheet_pr(source: &str) -> Result<Option<(usize, usize, usize, usize)>> {
    let Some(start) = find_open_tag(source, "sheetPr") else {
        return Ok(None);
    };
    let open_end = source[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| writer_error("unterminated sheetPr"))?;
    if source[start..open_end].ends_with("/>") {
        return Ok(Some((start, open_end, open_end, open_end)));
    }
    let close = "</sheetPr>";
    let close_start = source[open_end..]
        .find(close)
        .map(|offset| open_end + offset)
        .ok_or_else(|| writer_error("sheetPr is missing a closing tag"))?;
    Ok(Some((
        start,
        open_end,
        close_start,
        close_start + close.len(),
    )))
}

fn find_page_setup_pr(inner: &str) -> Result<Option<(usize, usize)>> {
    let Some(start) = find_open_tag(inner, "pageSetUpPr") else {
        return Ok(None);
    };
    let open_end = inner[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| writer_error("unterminated pageSetUpPr"))?;
    if inner[start..open_end].ends_with("/>") {
        return Ok(Some((start, open_end)));
    }
    let close = "</pageSetUpPr>";
    let end = inner[open_end..]
        .find(close)
        .map(|offset| open_end + offset + close.len())
        .ok_or_else(|| writer_error("missing closing </pageSetUpPr>"))?;
    Ok(Some((start, end)))
}

fn sheet_pr_insertion_point(source: &str) -> Result<usize> {
    if let Some(offset) = source.find("<dimension") {
        return Ok(offset);
    }
    if let Some(offset) = source.find("<sheetViews") {
        return Ok(offset);
    }
    if let Some(offset) = source.find("<sheetData") {
        return Ok(offset);
    }
    source
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| writer_error("worksheet XML is missing root tag"))
}

fn find_open_tag(source: &str, local: &str) -> Option<usize> {
    let mut cursor = 0;
    let open = format!("<{local}");
    while let Some(rel) = source[cursor..].find(&open) {
        let start = cursor + rel;
        let after = source.as_bytes().get(start + open.len()).copied()?;
        if after == b' ' || after == b'>' || after == b'/' {
            return Some(start);
        }
        cursor = start + open.len();
    }
    None
}

fn writer_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx fit_to_page writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_inserts_sheet_pr_and_page_setup() {
        let xml = br#"<worksheet><sheetData/><pageMargins/></worksheet>"#;
        let patched = patch(xml, Some(1), Some(2)).expect("upsert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"fitToPage="1""#), "unexpected XML: {text}");
        assert!(
            text.contains(r#"fitToWidth="1""#) && text.contains(r#"fitToHeight="2""#),
            "unexpected XML: {text}"
        );
    }

    #[test]
    fn clear_removes_fit_attrs() {
        let xml = br#"<worksheet><sheetPr><pageSetUpPr fitToPage="1"/></sheetPr><sheetData/><pageSetup fitToWidth="1" fitToHeight="1"/><pageMargins/></worksheet>"#;
        let patched = patch(xml, None, None).expect("clear");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(!text.contains("fitToWidth"), "unexpected XML: {text}");
        assert!(!text.contains("fitToHeight"), "unexpected XML: {text}");
    }
}
