use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;

/// Set or clear worksheet `<autoFilter ref="…"/>`. `None` clears.
pub(super) fn patch(xml: &[u8], range: Option<&str>) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    match range {
        Some(reference) => upsert_auto_filter(source, reference),
        None => clear_auto_filter(source),
    }
}

fn upsert_auto_filter(source: &str, reference: &str) -> Result<Vec<u8>> {
    let tag = format!(r#"<autoFilter ref="{reference}"/>"#);
    if let Some((start, end)) = find_auto_filter(source)? {
        let mut output = String::with_capacity(source.len() + tag.len());
        output.push_str(&source[..start]);
        output.push_str(&tag);
        output.push_str(&source[end..]);
        return Ok(output.into_bytes());
    }
    let insertion =
        sheet_data_end(source).ok_or_else(|| writer_error("worksheet XML is missing sheetData"))?;
    let mut output = String::with_capacity(source.len() + tag.len());
    output.push_str(&source[..insertion]);
    output.push_str(&tag);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn sheet_data_end(source: &str) -> Option<usize> {
    if let Some(offset) = source.find("</sheetData>") {
        return Some(offset + "</sheetData>".len());
    }
    // Self-closing <sheetData/> (or with attributes).
    let mut cursor = 0;
    let open = "<sheetData";
    while let Some(rel) = source[cursor..].find(open) {
        let start = cursor + rel;
        let after = source.as_bytes().get(start + open.len()).copied()?;
        if after != b' ' && after != b'>' && after != b'/' {
            cursor = start + open.len();
            continue;
        }
        let open_end = source[start..].find('>').map(|offset| start + offset + 1)?;
        if source[start..open_end].ends_with("/>") {
            return Some(open_end);
        }
        return source[open_end..]
            .find("</sheetData>")
            .map(|offset| open_end + offset + "</sheetData>".len());
    }
    None
}

fn clear_auto_filter(source: &str) -> Result<Vec<u8>> {
    let Some((start, end)) = find_auto_filter(source)? else {
        return Ok(source.as_bytes().to_vec());
    };
    let mut output = String::with_capacity(source.len());
    output.push_str(&source[..start]);
    output.push_str(&source[end..]);
    Ok(output.into_bytes())
}

fn find_auto_filter(source: &str) -> Result<Option<(usize, usize)>> {
    let Some(start) = find_open_tag(source, "autoFilter") else {
        return Ok(None);
    };
    let open_end = source[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| writer_error("unterminated autoFilter"))?;
    if source[start..open_end].ends_with("/>") {
        return Ok(Some((start, open_end)));
    }
    let close = "</autoFilter>";
    let end = source[open_end..]
        .find(close)
        .map(|offset| open_end + offset + close.len())
        .ok_or_else(|| writer_error("missing closing </autoFilter>"))?;
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
        path: "<xlsx auto_filter writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_inserts_after_sheet_data() {
        let xml = br#"<worksheet><sheetData/><mergeCells count="1"><mergeCell ref="A1:B1"/></mergeCells></worksheet>"#;
        let patched = patch(xml, Some("A1:C3")).expect("upsert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(
            text.contains(r#"<sheetData/><autoFilter ref="A1:C3"/><mergeCells"#)
                || text.contains(r#"</sheetData><autoFilter ref="A1:C3"/><mergeCells"#),
            "unexpected XML: {text}"
        );
    }

    #[test]
    fn clear_removes_auto_filter() {
        let xml = br#"<worksheet><sheetData/><autoFilter ref="A1:B2"/><mergeCells/></worksheet>"#;
        let patched = patch(xml, None).expect("clear");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(!text.contains("autoFilter"));
        assert!(text.contains("<mergeCells/>"));
    }
}
