use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use dotall_core::{
    DependencyImpact, DotallError, PatchedOutput, Result, SemanticChange, SemanticOperation,
    ValidatedEdit,
};
use zip::ZipArchive;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::FORMAT_ID;
use crate::model::{PresentationModel, SCHEMA_ID, SCHEMA_VERSION, TableCellModel, TableModel};
use crate::selector;

pub fn validate(
    model: &PresentationModel,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "pptx supports a single edit operation per transaction",
        ));
    }
    let operation = &operations[0];
    match operation.kind.as_str() {
        "set_shape_text" => validate_set_shape_text(model, operation),
        "set_table_cell_text" => validate_set_table_cell_text(model, operation),
        other => Err(format_error(format!(
            "unsupported pptx edit `{other}`; use set_shape_text or set_table_cell_text"
        ))),
    }
}

fn validate_set_shape_text(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let slide_ref = required_str(&operation.payload, "slide")?;
    let shape_ref = required_str(&operation.payload, "shape")?;
    let text = required_str(&operation.payload, "text")?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    let shape = slide
        .shapes
        .iter()
        .find(|shape| shape.name == shape_ref || shape.element_id == shape_ref)
        .ok_or_else(|| format_error(format!("shape `{shape_ref}` was not found")))?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_shape_text".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "shape": shape.name,
                "text": text,
                "part_name": slide.part_name,
                "element_id": shape.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}!{}", slide.name, shape.name),
            element_id: shape.element_id.clone(),
            change: "set_shape_text".into(),
            before: Some(shape.text.clone()),
            after: Some(text.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_table_cell_text(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let slide_ref = required_str(&operation.payload, "slide")?;
    let table_ref = table_ref(&operation.payload)?;
    let row = required_u32(&operation.payload, "row")?;
    let col = required_u32(&operation.payload, "col")?;
    let text = required_str(&operation.payload, "text")?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    let table = resolve_table(slide, table_ref)
        .ok_or_else(|| format_error(format!("table `{table_ref}` was not found")))?;
    let cell = resolve_cell(table, row, col).ok_or_else(|| {
        format_error(format!(
            "table cell row={row} col={col} is out of range for `{}`",
            table.name
        ))
    })?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_table_cell_text".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "table": table.name,
                "row": row,
                "col": col,
                "text": text,
                "part_name": slide.part_name,
                "element_id": cell.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}!{}!r{}c{}", slide.name, table.name, row, col),
            element_id: cell.element_id.clone(),
            change: "set_table_cell_text".into(),
            before: Some(cell.text.clone()),
            after: Some(text.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn resolve_table<'a>(
    slide: &'a crate::model::SlideModel,
    table_ref: &str,
) -> Option<&'a TableModel> {
    slide
        .tables
        .iter()
        .find(|table| table.name == table_ref || table.element_id == table_ref)
}

fn resolve_cell(table: &TableModel, row: u32, col: u32) -> Option<&TableCellModel> {
    table
        .cells
        .iter()
        .find(|cell| cell.row == row && cell.col == col)
}

pub fn apply_bytes(package: &[u8], edit: &ValidatedEdit) -> Result<PatchedOutput> {
    let operation = edit
        .operations
        .first()
        .ok_or_else(|| format_error("validated edit is missing operations"))?;
    let part_name = required_str(&operation.payload, "part_name")?;
    let text = required_str(&operation.payload, "text")?;
    let original = entry_bytes(package, part_name)?;
    let patched_xml = match operation.kind.as_str() {
        "set_shape_text" => {
            let shape = required_str(&operation.payload, "shape")?;
            patch_shape_text(&original, shape, text)?
        }
        "set_table_cell_text" => {
            let table = required_str(&operation.payload, "table")?;
            let row = required_u32(&operation.payload, "row")?;
            let col = required_u32(&operation.payload, "col")?;
            patch_table_cell_text(&original, table, row, col, text)?
        }
        other => {
            return Err(format_error(format!(
                "cannot apply unsupported pptx edit `{other}`"
            )));
        }
    };
    let bytes = rebuild_package(
        package,
        &BTreeMap::from([(part_name.to_owned(), patched_xml)]),
    )?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

pub fn patch_shape_text(xml: &[u8], shape: &str, text: &str) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("slide XML is not UTF-8: {error}")))?;
    let needle = format!("name=\"{shape}\"");
    let name_at = source
        .find(&needle)
        .ok_or_else(|| format_error(format!("shape `{shape}` was not found in slide XML")))?;
    let sp_start = source[..name_at]
        .rfind("<p:sp")
        .ok_or_else(|| format_error("shape is missing a p:sp wrapper"))?;
    let rel_end = source[name_at..]
        .find("</p:sp>")
        .ok_or_else(|| format_error("shape is missing a closing p:sp"))?;
    let sp_end = name_at + rel_end + "</p:sp>".len();
    let sp = &source[sp_start..sp_end];
    if sp.contains("<p:graphicFrame")
        || sp.contains("<a:graphic")
        || sp.contains("<mc:AlternateContent")
    {
        return Err(format_error("cannot edit non-text shape"));
    }
    let Some(t_rel) = find_tag(sp, 0, "a:t") else {
        return Err(format_error("shape has no text run to patch"));
    };
    let t_open_end = sp[t_rel..]
        .find('>')
        .map(|offset| t_rel + offset + 1)
        .ok_or_else(|| format_error("unterminated a:t"))?;
    let t_close_rel = sp[t_open_end..]
        .find("</a:t>")
        .ok_or_else(|| format_error("unterminated a:t content"))?;
    let t_close = t_open_end + t_close_rel;
    let mut patched_sp = String::new();
    patched_sp.push_str(&sp[..t_open_end]);
    patched_sp.push_str(&xml_escape(text));
    patched_sp.push_str(&sp[t_close..]);
    // Drop extra a:t contents after the first run.
    patched_sp = clear_later_text_runs(&patched_sp);
    let mut output = String::new();
    output.push_str(&source[..sp_start]);
    output.push_str(&patched_sp);
    output.push_str(&source[sp_end..]);
    Ok(output.into_bytes())
}

pub fn patch_table_cell_text(
    xml: &[u8],
    table: &str,
    row: u32,
    col: u32,
    text: &str,
) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("slide XML is not UTF-8: {error}")))?;
    let needle = format!("name=\"{table}\"");
    let name_at = source
        .find(&needle)
        .ok_or_else(|| format_error(format!("table `{table}` was not found in slide XML")))?;
    let frame_start = source[..name_at]
        .rfind("<p:graphicFrame")
        .ok_or_else(|| format_error("table is missing a p:graphicFrame wrapper"))?;
    let rel_end = source[name_at..]
        .find("</p:graphicFrame>")
        .ok_or_else(|| format_error("table is missing a closing p:graphicFrame"))?;
    let frame_end = name_at + rel_end + "</p:graphicFrame>".len();
    let frame = &source[frame_start..frame_end];
    let tbl_rel = find_tag(frame, 0, "a:tbl")
        .ok_or_else(|| format_error(format!("graphicFrame `{table}` has no a:tbl")))?;
    let tbl_close_rel = frame[tbl_rel..]
        .find("</a:tbl>")
        .ok_or_else(|| format_error("unterminated a:tbl"))?;
    let tbl_end = tbl_rel + tbl_close_rel + "</a:tbl>".len();
    let tbl = &frame[tbl_rel..tbl_end];
    let cell = nth_table_cell(tbl, row, col)?;
    let Some(t_rel) = find_tag(cell, 0, "a:t") else {
        return Err(format_error("table cell has no text run to patch"));
    };
    let t_open_end = cell[t_rel..]
        .find('>')
        .map(|offset| t_rel + offset + 1)
        .ok_or_else(|| format_error("unterminated a:t"))?;
    let t_close_rel = cell[t_open_end..]
        .find("</a:t>")
        .ok_or_else(|| format_error("unterminated a:t content"))?;
    let t_close = t_open_end + t_close_rel;
    let mut patched_cell = String::new();
    patched_cell.push_str(&cell[..t_open_end]);
    patched_cell.push_str(&xml_escape(text));
    patched_cell.push_str(&cell[t_close..]);
    patched_cell = clear_later_text_runs(&patched_cell);

    let cell_abs_start = frame_start + tbl_rel + cell_offset_in_tbl(tbl, row, col)?;
    let cell_abs_end = cell_abs_start + cell.len();
    let mut output = String::new();
    output.push_str(&source[..cell_abs_start]);
    output.push_str(&patched_cell);
    output.push_str(&source[cell_abs_end..]);
    Ok(output.into_bytes())
}

fn nth_table_cell(tbl: &str, row: u32, col: u32) -> Result<&str> {
    let (start, end) = cell_span(tbl, row, col)?;
    Ok(&tbl[start..end])
}

fn cell_offset_in_tbl(tbl: &str, row: u32, col: u32) -> Result<usize> {
    let (start, _) = cell_span(tbl, row, col)?;
    Ok(start)
}

fn cell_span(tbl: &str, row: u32, col: u32) -> Result<(usize, usize)> {
    let mut cursor = 0usize;
    let mut current_row = 0u32;
    while let Some(tr_start) = find_tag(tbl, cursor, "a:tr") {
        let open_end = tbl[tr_start..]
            .find('>')
            .map(|offset| tr_start + offset + 1)
            .ok_or_else(|| format_error("unterminated a:tr"))?;
        let close_rel = tbl[open_end..]
            .find("</a:tr>")
            .ok_or_else(|| format_error("unterminated a:tr content"))?;
        let tr_end = open_end + close_rel + "</a:tr>".len();
        if current_row == row {
            let tr = &tbl[tr_start..tr_end];
            let (cell_start, cell_end) = nth_cell_in_row(tr, col)?;
            return Ok((tr_start + cell_start, tr_start + cell_end));
        }
        current_row += 1;
        cursor = tr_end;
    }
    Err(format_error(format!(
        "table cell row={row} col={col} is out of range"
    )))
}

fn nth_cell_in_row(tr: &str, col: u32) -> Result<(usize, usize)> {
    let mut cursor = 0usize;
    let mut current_col = 0u32;
    while let Some(tc_start) = find_tag(tr, cursor, "a:tc") {
        let open_end = tr[tc_start..]
            .find('>')
            .map(|offset| tc_start + offset + 1)
            .ok_or_else(|| format_error("unterminated a:tc"))?;
        let close_rel = tr[open_end..]
            .find("</a:tc>")
            .ok_or_else(|| format_error("unterminated a:tc content"))?;
        let tc_end = open_end + close_rel + "</a:tc>".len();
        if current_col == col {
            return Ok((tc_start, tc_end));
        }
        current_col += 1;
        cursor = tc_end;
    }
    Err(format_error(format!(
        "table cell row col={col} is out of range"
    )))
}

fn clear_later_text_runs(fragment: &str) -> String {
    let Some(first) = find_tag(fragment, 0, "a:t") else {
        return fragment.to_owned();
    };
    let Some(first_close_rel) = fragment[first..].find("</a:t>") else {
        return fragment.to_owned();
    };
    let mut cursor = first + first_close_rel + "</a:t>".len();
    let mut output = fragment[..cursor].to_owned();
    while let Some(start) = find_tag(fragment, cursor, "a:t") {
        let Some(open_end_rel) = fragment[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel + 1;
        let Some(close_rel) = fragment[open_end..].find("</a:t>") else {
            break;
        };
        output.push_str(&fragment[cursor..open_end]);
        output.push_str(&fragment[open_end + close_rel..open_end + close_rel + "</a:t>".len()]);
        cursor = open_end + close_rel + "</a:t>".len();
    }
    output.push_str(&fragment[cursor..]);
    output
}

/// Find `<local` / `<local …>` without matching longer names (e.g. `a:t` vs `a:tc`).
fn find_tag(haystack: &str, start: usize, local: &str) -> Option<usize> {
    let needle = format!("<{local}");
    let mut cursor = start;
    while let Some(rel) = haystack[cursor..].find(&needle) {
        let at = cursor + rel;
        let next = *haystack.as_bytes().get(at + needle.len())?;
        if next == b'>' || next == b' ' || next == b'/' {
            return Some(at);
        }
        cursor = at + needle.len();
    }
    None
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn entry_bytes(package: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(format!("invalid PPTX package: {error}")))?;
    let mut entry = archive
        .by_name(name)
        .map_err(|error| format_error(format!("missing `{name}`: {error}")))?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes)
        .map_err(|error| format_error(format!("cannot read `{name}`: {error}")))?;
    Ok(bytes)
}

fn rebuild_package(original: &[u8], replacements: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(original))
        .map_err(|error| format_error(format!("invalid PPTX package: {error}")))?;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format_error(format!("cannot read ZIP entry: {error}")))?;
        let name = entry.name().to_owned();
        if let Some(replacement) = replacements.get(&name) {
            let options = SimpleFileOptions::default()
                .compression_method(entry.compression())
                .last_modified_time(entry.last_modified().unwrap_or_default());
            writer.start_file(&name, options).map_err(|error| {
                format_error(format!("cannot start patched ZIP entry: {error}"))
            })?;
            writer.write_all(replacement).map_err(|error| {
                format_error(format!("cannot write patched ZIP entry: {error}"))
            })?;
        } else {
            writer
                .raw_copy_file(entry)
                .map_err(|error| format_error(format!("cannot copy ZIP entry: {error}")))?;
        }
    }
    writer
        .finish()
        .map_err(|error| format_error(format!("cannot finish PPTX package: {error}")))
        .map(|cursor| cursor.into_inner())
}

fn table_ref(payload: &serde_json::Value) -> Result<&str> {
    if let Ok(value) = required_str(payload, "table") {
        return Ok(value);
    }
    required_str(payload, "shape").map_err(|_| format_error("`table` is required"))
}

fn required_str<'a>(payload: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format_error(format!("`{key}` is required")))
}

fn required_u32(payload: &serde_json::Value, key: &str) -> Result<u32> {
    payload
        .get(key)
        .and_then(|value| {
            value
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .or_else(|| {
                    value
                        .as_i64()
                        .and_then(|n| if n >= 0 { u32::try_from(n).ok() } else { None })
                })
        })
        .ok_or_else(|| format_error(format!("`{key}` is required")))
}

pub fn apply(source: &std::path::Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
    let bytes = std::fs::read(source).map_err(|error| DotallError::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    apply_bytes(&bytes, edit)
}

fn format_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<pptx edit>".into(),
        message: message.into(),
    }
}
