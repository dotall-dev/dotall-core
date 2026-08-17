use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;

/// Set worksheet `<sheetView showGridLines="0"/>` without replacing freeze-pane children.
///
/// `show = false` writes `showGridLines="0"`. `show = true` removes the attribute (Excel default shown).
pub(super) fn patch(xml: &[u8], show: bool) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    upsert_show_gridlines(source, show)
}

fn upsert_show_gridlines(source: &str, show: bool) -> Result<Vec<u8>> {
    if let Some((view_start, view_open_end, _, _)) = find_sheet_view(source)? {
        let open_tag = &source[view_start..view_open_end];
        let patched_open = upsert_show_gridlines_attr(open_tag, show)?;
        let mut output = String::with_capacity(source.len() + patched_open.len());
        output.push_str(&source[..view_start]);
        output.push_str(&patched_open);
        output.push_str(&source[view_open_end..]);
        return Ok(output.into_bytes());
    }

    if show {
        return Ok(source.as_bytes().to_vec());
    }

    let sheet_views =
        r#"<sheetViews><sheetView workbookViewId="0" showGridLines="0"/></sheetViews>"#;
    let insertion = sheet_views_insertion_point(source)?;
    let mut output = String::with_capacity(source.len() + sheet_views.len());
    output.push_str(&source[..insertion]);
    output.push_str(sheet_views);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn upsert_show_gridlines_attr(open: &str, show: bool) -> Result<String> {
    let without = remove_attr(open, "showGridLines");
    if show {
        return Ok(without);
    }
    let self_closing = without.ends_with("/>");
    let base = if self_closing {
        without.trim_end_matches("/>").trim_end()
    } else {
        without.trim_end_matches('>').trim_end()
    };
    if self_closing {
        Ok(format!(r#"{base} showGridLines="0"/>"#))
    } else {
        Ok(format!(r#"{base} showGridLines="0">"#))
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
        path: "<xlsx show_gridlines writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hide_writes_show_grid_lines_zero_on_existing_sheet_view() {
        let xml = br#"<worksheet><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetData/></worksheet>"#;
        let patched = patch(xml, false).expect("hide");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"workbookViewId="0""#));
        assert!(text.contains(r#"showGridLines="0""#));
    }

    #[test]
    fn show_removes_show_grid_lines_attribute() {
        let xml = br#"<worksheet><sheetViews><sheetView workbookViewId="0" showGridLines="0" zoomScale="75"/></sheetViews><sheetData/></worksheet>"#;
        let patched = patch(xml, true).expect("show");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(!text.contains("showGridLines="));
        assert!(text.contains(r#"workbookViewId="0""#));
        assert!(text.contains(r#"zoomScale="75""#));
    }

    #[test]
    fn hide_preserves_freeze_pane_children_and_zoom() {
        let xml = br#"<worksheet><sheetViews><sheetView workbookViewId="0" zoomScale="75"><pane xSplit="1" ySplit="1" topLeftCell="B2" activePane="bottomRight" state="frozen"/><selection pane="bottomRight"/></sheetView></sheetViews><sheetData/></worksheet>"#;
        let patched = patch(xml, false).expect("hide");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"showGridLines="0""#));
        assert!(text.contains(r#"zoomScale="75""#));
        assert!(text.contains(r#"workbookViewId="0""#));
        assert!(text.contains(r#"<pane xSplit="1" ySplit="1" topLeftCell="B2""#));
        assert!(text.contains(r#"<selection pane="bottomRight"/>"#));
    }

    #[test]
    fn inserts_sheet_views_when_missing_and_hiding() {
        let xml = br#"<worksheet><sheetData><row r="1"/></sheetData></worksheet>"#;
        let patched = patch(xml, false).expect("insert");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(
            r#"<sheetViews><sheetView workbookViewId="0" showGridLines="0"/></sheetViews><sheetData"#
        ));
    }
}
