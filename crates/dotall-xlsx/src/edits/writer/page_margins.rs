use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;
use crate::model::PageMargins;

/// Upsert worksheet `<pageMargins …/>` with the given inch values.
pub(super) fn patch(xml: &[u8], margins: &PageMargins) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    upsert_page_margins(source, margins)
}

fn upsert_page_margins(source: &str, margins: &PageMargins) -> Result<Vec<u8>> {
    let tag = render_page_margins(margins);
    if let Some((start, end)) = find_page_margins(source)? {
        let mut output = String::with_capacity(source.len() + tag.len());
        output.push_str(&source[..start]);
        output.push_str(&tag);
        output.push_str(&source[end..]);
        return Ok(output.into_bytes());
    }
    let insertion = insert_page_margins_at(source)
        .ok_or_else(|| writer_error("worksheet XML is missing closing worksheet tag"))?;
    let mut output = String::with_capacity(source.len() + tag.len());
    output.push_str(&source[..insertion]);
    output.push_str(&tag);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn render_page_margins(margins: &PageMargins) -> String {
    let mut tag = format!(
        r#"<pageMargins left="{}" right="{}" top="{}" bottom="{}""#,
        format_margin(margins.left),
        format_margin(margins.right),
        format_margin(margins.top),
        format_margin(margins.bottom),
    );
    if let Some(header) = margins.header {
        tag.push_str(&format!(r#" header="{}""#, format_margin(header)));
    }
    if let Some(footer) = margins.footer {
        tag.push_str(&format!(r#" footer="{}""#, format_margin(footer)));
    }
    tag.push_str("/>");
    tag
}

fn format_margin(value: f64) -> String {
    if (value - value.round()).abs() < 1e-12 {
        format!("{}", value.round() as i64)
    } else {
        let formatted = format!("{value:.10}");
        formatted
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn insert_page_margins_at(source: &str) -> Option<usize> {
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

fn find_page_margins(source: &str) -> Result<Option<(usize, usize)>> {
    let Some(start) = find_open_tag(source, "pageMargins") else {
        return Ok(None);
    };
    let open_end = source[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| writer_error("unterminated pageMargins"))?;
    if source[start..open_end].ends_with("/>") {
        return Ok(Some((start, open_end)));
    }
    let close = "</pageMargins>";
    let end = source[open_end..]
        .find(close)
        .map(|offset| open_end + offset + close.len())
        .ok_or_else(|| writer_error("missing closing </pageMargins>"))?;
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
        path: "<xlsx page_margins writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_inserts_before_page_setup() {
        let xml = br#"<worksheet><sheetData/><pageSetup orientation="landscape"/></worksheet>"#;
        let patched = patch(
            xml,
            &PageMargins {
                left: 0.5,
                right: 0.5,
                top: 0.6,
                bottom: 0.6,
                header: Some(0.25),
                footer: Some(0.25),
            },
        )
        .expect("upsert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(
            text.contains(r#"left="0.5""#) && text.contains(r#"header="0.25""#),
            "unexpected XML: {text}"
        );
        assert!(text.contains(r#"orientation="landscape""#));
    }

    #[test]
    fn upsert_replaces_existing() {
        let xml = br#"<worksheet><sheetData/><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75"/></worksheet>"#;
        let patched = patch(
            xml,
            &PageMargins {
                left: 1.0,
                right: 1.0,
                top: 1.0,
                bottom: 1.0,
                header: None,
                footer: None,
            },
        )
        .expect("upsert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"left="1""#), "unexpected XML: {text}");
        assert!(!text.contains(r#"left="0.7""#));
    }
}
