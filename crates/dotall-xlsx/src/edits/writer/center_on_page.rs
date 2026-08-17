use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;
use crate::model::CenterOnPage;

/// Set or clear worksheet print centering via `<printOptions>`.
pub(super) fn patch(xml: &[u8], center: &CenterOnPage) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    if !center.horizontal && !center.vertical {
        return clear_print_options(source);
    }
    upsert_print_options(source, center)
}

fn clear_print_options(source: &str) -> Result<Vec<u8>> {
    if let Some((start, end)) = find_print_options(source)? {
        let mut output = String::with_capacity(source.len());
        output.push_str(&source[..start]);
        output.push_str(&source[end..]);
        return Ok(output.into_bytes());
    }
    Ok(source.as_bytes().to_vec())
}

fn upsert_print_options(source: &str, center: &CenterOnPage) -> Result<Vec<u8>> {
    let tag = render_print_options(center);
    if let Some((start, end)) = find_print_options(source)? {
        let mut output = String::with_capacity(source.len() + tag.len());
        output.push_str(&source[..start]);
        output.push_str(&tag);
        output.push_str(&source[end..]);
        return Ok(output.into_bytes());
    }
    let insertion = insert_print_options_at(source)
        .ok_or_else(|| writer_error("worksheet XML is missing closing worksheet tag"))?;
    let mut output = String::with_capacity(source.len() + tag.len());
    output.push_str(&source[..insertion]);
    output.push_str(&tag);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn render_print_options(center: &CenterOnPage) -> String {
    let h = if center.horizontal { "1" } else { "0" };
    let v = if center.vertical { "1" } else { "0" };
    format!(r#"<printOptions horizontalCentered="{h}" verticalCentered="{v}"/>"#)
}

fn insert_print_options_at(source: &str) -> Option<usize> {
    if let Some(offset) = source.find("<pageMargins") {
        return Some(offset);
    }
    if let Some(offset) = source.find("<pageSetup") {
        return Some(offset);
    }
    if let Some(offset) = source.find("<headerFooter") {
        return Some(offset);
    }
    if let Some(offset) = source.rfind("</worksheet>") {
        return Some(offset);
    }
    None
}

fn find_print_options(source: &str) -> Result<Option<(usize, usize)>> {
    let Some(start) = find_open_tag(source, "printOptions") else {
        return Ok(None);
    };
    let open_end = source[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| writer_error("unterminated printOptions"))?;
    if source[start..open_end].ends_with("/>") {
        return Ok(Some((start, open_end)));
    }
    let close = "</printOptions>";
    let end = source[open_end..]
        .find(close)
        .map(|offset| open_end + offset + close.len())
        .ok_or_else(|| writer_error("missing closing </printOptions>"))?;
    Ok(Some((start, end)))
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
        path: "<xlsx center_on_page writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_inserts_print_options() {
        let xml = br#"<worksheet><sheetData/><pageMargins/></worksheet>"#;
        let patched = patch(
            xml,
            &CenterOnPage {
                horizontal: true,
                vertical: false,
            },
        )
        .expect("upsert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(
            text.contains(r#"horizontalCentered="1""#) && text.contains(r#"verticalCentered="0""#),
            "unexpected XML: {text}"
        );
    }

    #[test]
    fn clear_removes_print_options() {
        let xml = br#"<worksheet><sheetData/><printOptions horizontalCentered="1" verticalCentered="1"/><pageMargins/></worksheet>"#;
        let patched = patch(
            xml,
            &CenterOnPage {
                horizontal: false,
                vertical: false,
            },
        )
        .expect("clear");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(!text.contains("printOptions"), "unexpected XML: {text}");
    }
}
