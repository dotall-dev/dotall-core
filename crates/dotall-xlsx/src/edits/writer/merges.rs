use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;
use crate::edits::transform::parse_range;

pub(super) enum MergeEdit {
    Merge { range: String },
    Unmerge { range: String },
}

pub(super) fn patch(xml: &[u8], edit: &MergeEdit) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    match edit {
        MergeEdit::Merge { range } => add_merge(source, range),
        MergeEdit::Unmerge { range } => remove_merge(source, range),
    }
}

fn add_merge(source: &str, range: &str) -> Result<Vec<u8>> {
    if let Some((block_start, block_end, merges)) = parse_merge_cells_block(source)? {
        let mut refs = merges;
        if refs
            .iter()
            .any(|existing| canonicalize_ref(existing).as_deref() == Some(range))
        {
            return Err(writer_error(format!(
                "merge `{range}` already exists in worksheet"
            )));
        }
        refs.push(range.to_owned());
        let replacement = render_merge_cells(&refs);
        let mut output = String::with_capacity(source.len() + replacement.len());
        output.push_str(&source[..block_start]);
        output.push_str(&replacement);
        output.push_str(&source[block_end..]);
        return Ok(output.into_bytes());
    }

    let insertion = source
        .find("</sheetData>")
        .map(|offset| offset + "</sheetData>".len())
        .ok_or_else(|| writer_error("worksheet XML is missing </sheetData>"))?;
    let block = render_merge_cells(&[range.to_owned()]);
    let mut output = String::with_capacity(source.len() + block.len());
    output.push_str(&source[..insertion]);
    output.push_str(&block);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn remove_merge(source: &str, range: &str) -> Result<Vec<u8>> {
    let (block_start, block_end, merges) = parse_merge_cells_block(source)?
        .ok_or_else(|| writer_error(format!("merge `{range}` was not found in worksheet")))?;
    let before_len = merges.len();
    let remaining = merges
        .into_iter()
        .filter(|existing| canonicalize_ref(existing).as_deref() != Some(range))
        .collect::<Vec<_>>();
    if remaining.len() == before_len {
        return Err(writer_error(format!(
            "merge `{range}` was not found in worksheet"
        )));
    }
    let replacement = if remaining.is_empty() {
        String::new()
    } else {
        render_merge_cells(&remaining)
    };
    let mut output = String::with_capacity(source.len());
    output.push_str(&source[..block_start]);
    output.push_str(&replacement);
    output.push_str(&source[block_end..]);
    Ok(output.into_bytes())
}

fn parse_merge_cells_block(source: &str) -> Result<Option<(usize, usize, Vec<String>)>> {
    let Some(start) = find_open_tag(source, "mergeCells") else {
        return Ok(None);
    };
    let open_end = source[start..]
        .find('>')
        .map(|offset| start + offset)
        .ok_or_else(|| writer_error("unterminated mergeCells"))?;
    let open_tag = &source[start..=open_end];
    if open_tag.ends_with("/>") {
        return Ok(Some((start, open_end + 1, Vec::new())));
    }
    let close_needle = "</mergeCells>";
    let close = source[open_end + 1..]
        .find(close_needle)
        .map(|offset| open_end + 1 + offset)
        .or_else(|| {
            source[open_end + 1..]
                .find("</x:mergeCells>")
                .map(|offset| open_end + 1 + offset)
        })
        .ok_or_else(|| writer_error("unterminated mergeCells element"))?;
    let close_len = if source[close..].starts_with(close_needle) {
        close_needle.len()
    } else {
        "</x:mergeCells>".len()
    };
    let body = &source[open_end + 1..close];
    let mut merges = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = find_open_tag(&body[cursor..], "mergeCell") {
        let tag_start = cursor + relative;
        let tag_end = body[tag_start..]
            .find('>')
            .map(|offset| tag_start + offset)
            .ok_or_else(|| writer_error("unterminated mergeCell"))?;
        let tag = &body[tag_start..=tag_end];
        let reference = attribute(tag, "ref")
            .ok_or_else(|| writer_error("mergeCell is missing required `ref` attribute"))?;
        merges.push(reference.to_owned());
        cursor = tag_end + 1;
    }
    Ok(Some((start, close + close_len, merges)))
}

fn render_merge_cells(refs: &[String]) -> String {
    let mut output = format!(r#"<mergeCells count="{}">"#, refs.len());
    for reference in refs {
        output.push_str(&format!(r#"<mergeCell ref="{reference}"/>"#));
    }
    output.push_str("</mergeCells>");
    output
}

fn canonicalize_ref(reference: &str) -> Option<String> {
    parse_range(reference).ok().map(|range| {
        format!(
            "{}{}:{}{}",
            range.start.column, range.start.row, range.end.column, range.end.row
        )
    })
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
        path: "<xlsx merge writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_first_merge_after_sheet_data() {
        let xml = br#"<worksheet><sheetData><row r="1"/></sheetData><pageMargins/></worksheet>"#;
        let patched = patch(
            xml,
            &MergeEdit::Merge {
                range: "A1:B2".into(),
            },
        )
        .expect("add merge");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"<mergeCells count="1"><mergeCell ref="A1:B2"/></mergeCells>"#));
        assert!(text.contains("</sheetData><mergeCells"));
    }

    #[test]
    fn appends_to_existing_merge_cells() {
        let xml = br#"<worksheet><sheetData/><mergeCells count="1"><mergeCell ref="A1:B1"/></mergeCells></worksheet>"#;
        let patched = patch(
            xml,
            &MergeEdit::Merge {
                range: "A2:B3".into(),
            },
        )
        .expect("add merge");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"count="2""#));
        assert!(text.contains(r#"ref="A1:B1""#));
        assert!(text.contains(r#"ref="A2:B3""#));
    }

    #[test]
    fn removes_last_merge_cells_block() {
        let xml = br#"<worksheet><sheetData/><mergeCells count="1"><mergeCell ref="A1:B1"/></mergeCells><pageMargins/></worksheet>"#;
        let patched = patch(
            xml,
            &MergeEdit::Unmerge {
                range: "A1:B1".into(),
            },
        )
        .expect("unmerge");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(!text.contains("mergeCell"));
        assert!(text.contains("<sheetData/><pageMargins/>"));
    }
}
