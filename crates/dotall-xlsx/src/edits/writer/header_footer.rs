use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;

/// Set or clear worksheet `<headerFooter>` / `<oddHeader>` / `<oddFooter>`.
pub(super) fn patch(xml: &[u8], header: Option<&str>, footer: Option<&str>) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    if header.is_none() && footer.is_none() {
        return clear_header_footer(source);
    }
    upsert_header_footer(source, header, footer)
}

fn clear_header_footer(source: &str) -> Result<Vec<u8>> {
    if let Some((start, end)) = find_header_footer(source)? {
        let mut output = String::with_capacity(source.len());
        output.push_str(&source[..start]);
        output.push_str(&source[end..]);
        return Ok(output.into_bytes());
    }
    Ok(source.as_bytes().to_vec())
}

fn upsert_header_footer(
    source: &str,
    header: Option<&str>,
    footer: Option<&str>,
) -> Result<Vec<u8>> {
    let tag = render_header_footer(header, footer);
    if let Some((start, end)) = find_header_footer(source)? {
        let mut output = String::with_capacity(source.len() + tag.len());
        output.push_str(&source[..start]);
        output.push_str(&tag);
        output.push_str(&source[end..]);
        return Ok(output.into_bytes());
    }
    let insertion = insert_header_footer_at(source)?
        .ok_or_else(|| writer_error("worksheet XML is missing closing worksheet tag"))?;
    let mut output = String::with_capacity(source.len() + tag.len());
    output.push_str(&source[..insertion]);
    output.push_str(&tag);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn render_header_footer(header: Option<&str>, footer: Option<&str>) -> String {
    let mut body = String::new();
    if let Some(header) = header {
        body.push_str("<oddHeader>");
        body.push_str(&escape_xml(header));
        body.push_str("</oddHeader>");
    }
    if let Some(footer) = footer {
        body.push_str("<oddFooter>");
        body.push_str(&escape_xml(footer));
        body.push_str("</oddFooter>");
    }
    format!("<headerFooter>{body}</headerFooter>")
}

fn escape_xml(value: &str) -> String {
    quick_xml::escape::escape(value).into_owned()
}

fn insert_header_footer_at(source: &str) -> Result<Option<usize>> {
    if let Some((_, end)) = find_page_setup(source)? {
        return Ok(Some(end));
    }
    Ok(source.rfind("</worksheet>"))
}

fn find_header_footer(source: &str) -> Result<Option<(usize, usize)>> {
    find_element(source, "headerFooter")
}

fn find_page_setup(source: &str) -> Result<Option<(usize, usize)>> {
    find_element(source, "pageSetup")
}

fn find_element(source: &str, local: &str) -> Result<Option<(usize, usize)>> {
    let Some(start) = find_open_tag(source, local) else {
        return Ok(None);
    };
    let open_end = source[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| writer_error(format!("unterminated {local}")))?;
    if source[start..open_end].ends_with("/>") {
        return Ok(Some((start, open_end)));
    }
    let close = format!("</{local}>");
    let end = source[open_end..]
        .find(&close)
        .map(|offset| open_end + offset + close.len())
        .ok_or_else(|| writer_error(format!("missing closing </{local}>")))?;
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
        path: "<xlsx header_footer writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_inserts_after_page_setup_and_escapes_ampersands() {
        let xml = br#"<worksheet><sheetData/><pageSetup paperSize="1"/></worksheet>"#;
        let patched = patch(xml, Some("&CBoard pack"), Some("&P")).expect("upsert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(
            text.contains("<oddHeader>&amp;CBoard pack</oddHeader>")
                && text.contains("<oddFooter>&amp;P</oddFooter>"),
            "unexpected XML: {text}"
        );
        let setup_at = text.find("<pageSetup").expect("pageSetup");
        let header_at = text.find("<headerFooter>").expect("headerFooter");
        assert!(setup_at < header_at, "headerFooter must follow pageSetup");
    }

    #[test]
    fn clear_removes_header_footer() {
        let xml = br#"<worksheet><sheetData/><headerFooter><oddHeader>&amp;C</oddHeader></headerFooter></worksheet>"#;
        let patched = patch(xml, None, None).expect("clear");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(!text.contains("headerFooter"), "unexpected XML: {text}");
    }
}
