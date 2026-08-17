use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;
use crate::model::column_name;
use crate::selector::{self, CellAddress};

/// Openpyxl-style freeze cell. `None` / `A1` clears freeze panes.
pub(super) fn patch(xml: &[u8], cell: Option<&str>) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    let freeze = match cell {
        None => None,
        Some(value) => {
            let address = parse_freeze_cell(value)?;
            if address.row == 1 && address.col == 1 {
                None
            } else {
                Some(address)
            }
        }
    };
    apply_freeze(source, freeze)
}

fn parse_freeze_cell(value: &str) -> Result<CellAddress> {
    selector::parse_cell_address(value).map_err(writer_error)
}

fn apply_freeze(source: &str, freeze: Option<CellAddress>) -> Result<Vec<u8>> {
    let rendered_inner = match freeze {
        None => String::new(),
        Some(address) => render_freeze_children(address),
    };

    if let Some((view_start, view_open_end, _view_close_start, view_end)) = find_sheet_view(source)?
    {
        let open_tag = &source[view_start..view_open_end];
        let mut rebuilt = String::with_capacity(source.len() + rendered_inner.len() + 32);
        rebuilt.push_str(&source[..view_start]);
        if let Some(stripped) = open_tag.strip_suffix("/>") {
            let mut open = stripped.to_owned();
            open.push('>');
            rebuilt.push_str(&open);
            rebuilt.push_str(&rendered_inner);
            rebuilt.push_str("</sheetView>");
        } else {
            rebuilt.push_str(open_tag);
            rebuilt.push_str(&rendered_inner);
            rebuilt.push_str("</sheetView>");
        }
        rebuilt.push_str(&source[view_end..]);
        return Ok(rebuilt.into_bytes());
    }

    if freeze.is_none() {
        return Ok(source.as_bytes().to_vec());
    }

    let sheet_views = format!(
        r#"<sheetViews><sheetView workbookViewId="0">{rendered_inner}</sheetView></sheetViews>"#
    );
    let insertion = sheet_views_insertion_point(source)?;
    let mut output = String::with_capacity(source.len() + sheet_views.len());
    output.push_str(&source[..insertion]);
    output.push_str(&sheet_views);
    output.push_str(&source[insertion..]);
    Ok(output.into_bytes())
}

fn render_freeze_children(address: CellAddress) -> String {
    let x_split = address.col.saturating_sub(1);
    let y_split = address.row.saturating_sub(1);
    let top_left = format!("{}{}", column_name((address.col - 1) as usize), address.row);
    let mut out = String::new();
    out.push_str("<pane");
    if x_split > 0 {
        out.push_str(&format!(r#" xSplit="{x_split}""#));
    }
    if y_split > 0 {
        out.push_str(&format!(r#" ySplit="{y_split}""#));
    }
    out.push_str(&format!(r#" topLeftCell="{top_left}""#));
    let active = if x_split > 0 && y_split > 0 {
        "bottomRight"
    } else if y_split > 0 {
        "bottomLeft"
    } else {
        "topRight"
    };
    out.push_str(&format!(r#" activePane="{active}" state="frozen"/>"#));

    if x_split > 0 && y_split > 0 {
        let top_right = format!("{}1", column_name((address.col - 1) as usize));
        let bottom_left = format!("A{}", address.row);
        out.push_str(&format!(
            r#"<selection pane="topRight" activeCell="{top_right}" sqref="{top_right}"/>"#
        ));
        out.push_str(&format!(
            r#"<selection pane="bottomLeft" activeCell="{bottom_left}" sqref="{bottom_left}"/>"#
        ));
        out.push_str(r#"<selection pane="bottomRight"/>"#);
    } else if y_split > 0 {
        out.push_str(&format!(
            r#"<selection pane="bottomLeft" activeCell="{top_left}" sqref="{top_left}"/>"#
        ));
    } else {
        out.push_str(&format!(
            r#"<selection pane="topRight" activeCell="{top_left}" sqref="{top_left}"/>"#
        ));
    }
    out
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
        path: "<xlsx freeze_panes writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_frozen_pane_before_sheet_data() {
        let xml = br#"<worksheet><sheetData><row r="1"/></sheetData></worksheet>"#;
        let patched = patch(xml, Some("B2")).expect("freeze");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(text.contains(r#"topLeftCell="B2""#));
        assert!(text.contains(r#"state="frozen""#));
        assert!(text.contains("<sheetViews><sheetView"));
        assert!(text.contains("</sheetViews><sheetData"));
    }

    #[test]
    fn clears_existing_frozen_pane() {
        let xml = br#"<worksheet><sheetViews><sheetView workbookViewId="0"><pane xSplit="1" ySplit="1" topLeftCell="B2" activePane="bottomRight" state="frozen"/><selection pane="bottomRight"/></sheetView></sheetViews><sheetData/></worksheet>"#;
        let patched = patch(xml, None).expect("clear");
        let text = String::from_utf8(patched).expect("utf8");
        assert!(!text.contains(r#"state="frozen""#));
        assert!(text.contains(r#"<sheetView workbookViewId="0"></sheetView>"#));
    }
}
