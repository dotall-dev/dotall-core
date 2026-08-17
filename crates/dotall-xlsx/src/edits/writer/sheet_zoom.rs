use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;

/// Set worksheet `<sheetView zoomScale="N"/>` without replacing freeze-pane children.
pub(super) fn patch(xml: &[u8], zoom: u32) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    upsert_zoom_scale(source, zoom)
}

fn upsert_zoom_scale(source: &str, zoom: u32) -> Result<Vec<u8>> {
    if let Some((view_start, view_open_end, _, _)) = find_sheet_view(source)? {
        let open_tag = &source[view_start..view_open_end];
        let patched_open = upsert_zoom_scale_attr(open_tag, zoom)?;
        let mut output = String::with_capacity(source.len() + patched_open.len());
        output.push_str(&source[..view_start]);
        output.push_str(&patched_open);
        output.push_str(&source[view_open_end..]);
        return Ok(output.into_bytes());
    }

    let sheet_views =
        format!(r#"<sheetViews><sheetView workbookViewId="0" zoomScale="{zoom}"/></sheetViews>"#);
    let insertion = sheet_views_insertion_point(source)?;
    let mut output = String::with_capacity(source.len() + sheet_views.len());
    output.push_str(&source[..insertion]);
    output.push_str(&sheet_views);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn upsert_zoom_scale_attr(open: &str, zoom: u32) -> Result<String> {
    let without = remove_attr(open, "zoomScale");
    let self_closing = without.ends_with("/>");
    let base = if self_closing {
        without.trim_end_matches("/>").trim_end()
    } else {
        without.trim_end_matches('>').trim_end()
    };
    if self_closing {
        Ok(format!(r#"{base} zoomScale="{zoom}"/>"#))
    } else {
        Ok(format!(r#"{base} zoomScale="{zoom}">"#))
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

fn find_sheet_view(source: &str) -> Result<Option<(usize, usize, usize, usize)>> {
    let Some(view_start) = find_open_tag(source, "sheetView") else {
        return Ok(None);
    };
    let open_end = source[view_start..]
        .find('>')
        .map(|offset| view_start + offset + 1)
        .ok_or_else(|| writer_error("unterminated sheetView"))?;
    let open_tag = &source[view_start..open_end];
    if open_tag.ends_with("/>") {
        return Ok(Some((view_start, open_end, open_end, open_end)));
    }
    let close = source[open_end..]
        .find("</sheetView>")
        .map(|offset| open_end + offset)
        .or_else(|| {
            source[open_end..]
                .find("</x:sheetView>")
                .map(|offset| open_end + offset)
        })
        .ok_or_else(|| writer_error("unterminated sheetView element"))?;
    let close_len = if source[close..].starts_with("</x:sheetView>") {
        "</x:sheetView>".len()
    } else {
        "</sheetView>".len()
    };
    Ok(Some((view_start, open_end, close, close + close_len)))
}

fn sheet_views_insertion_point(source: &str) -> Result<usize> {
    if let Some(start) = find_open_tag(source, "sheetViews") {
        return Ok(start);
    }
    for local in ["sheetFormatPr", "cols", "sheetData"] {
        if let Some(start) = find_open_tag(source, local) {
            return Ok(start);
        }
    }
    Err(writer_error(
        "worksheet XML is missing sheetFormatPr/cols/sheetData for sheetViews insertion",
    ))
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

fn writer_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx sheet_zoom writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_inserts_zoom_on_existing_sheet_view() {
        let xml = br#"<worksheet><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetData/></worksheet>"#;
        let patched = patch(xml, 75).expect("upsert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"workbookViewId="0""#));
        assert!(text.contains(r#"zoomScale="75""#));
    }

    #[test]
    fn upsert_preserves_freeze_pane_children() {
        let xml = br#"<worksheet><sheetViews><sheetView workbookViewId="0"><pane xSplit="1" ySplit="1" topLeftCell="B2" activePane="bottomRight" state="frozen"/><selection pane="bottomRight"/></sheetView></sheetViews><sheetData/></worksheet>"#;
        let patched = patch(xml, 120).expect("upsert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"zoomScale="120""#));
        assert!(text.contains(r#"workbookViewId="0""#));
        assert!(text.contains(r#"<pane xSplit="1" ySplit="1" topLeftCell="B2""#));
        assert!(text.contains(r#"<selection pane="bottomRight"/>"#));
    }

    #[test]
    fn inserts_sheet_views_when_missing() {
        let xml = br#"<worksheet><sheetData><row r="1"/></sheetData></worksheet>"#;
        let patched = patch(xml, 50).expect("insert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(
            r#"<sheetViews><sheetView workbookViewId="0" zoomScale="50"/></sheetViews><sheetData"#
        ));
    }
}
