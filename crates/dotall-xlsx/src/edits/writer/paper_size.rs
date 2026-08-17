use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;

/// Set worksheet `<pageSetup paperSize="N"/>`.
pub(super) fn patch(xml: &[u8], paper_size: u32) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    upsert_page_setup(source, paper_size)
}

fn upsert_page_setup(source: &str, paper_size: u32) -> Result<Vec<u8>> {
    if let Some((start, end)) = find_page_setup(source)? {
        let existing = &source[start..end];
        let patched = upsert_paper_size_attr(existing, paper_size)?;
        let mut output = String::with_capacity(source.len() + patched.len());
        output.push_str(&source[..start]);
        output.push_str(&patched);
        output.push_str(&source[end..]);
        return Ok(output.into_bytes());
    }
    let tag = format!(r#"<pageSetup paperSize="{paper_size}"/>"#);
    let insertion = insert_page_setup_at(source)
        .ok_or_else(|| writer_error("worksheet XML is missing closing worksheet tag"))?;
    let mut output = String::with_capacity(source.len() + tag.len());
    output.push_str(&source[..insertion]);
    output.push_str(&tag);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn upsert_paper_size_attr(page_setup: &str, paper_size: u32) -> Result<String> {
    let open_end = page_setup
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| writer_error("unterminated pageSetup"))?;
    let open = &page_setup[..open_end];
    let rest = &page_setup[open_end..];
    let without = remove_attr(open, "paperSize");
    let self_closing = without.ends_with("/>");
    let base = if self_closing {
        without.trim_end_matches("/>").trim_end()
    } else {
        without.trim_end_matches('>').trim_end()
    };
    if self_closing {
        Ok(format!(r#"{base} paperSize="{paper_size}"/>"#))
    } else {
        Ok(format!(r#"{base} paperSize="{paper_size}">{rest}"#))
    }
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
        path: "<xlsx paper_size writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_inserts_paper_size() {
        let xml = br#"<worksheet><sheetData/><pageMargins/></worksheet>"#;
        let patched = patch(xml, 9).expect("upsert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"paperSize="9""#), "unexpected XML: {text}");
    }

    #[test]
    fn upsert_preserves_orientation() {
        let xml = br#"<worksheet><sheetData/><pageSetup orientation="landscape"/><pageMargins/></worksheet>"#;
        let patched = patch(xml, 1).expect("upsert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"orientation="landscape""#));
        assert!(text.contains(r#"paperSize="1""#));
    }
}
