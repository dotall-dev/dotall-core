use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use dotall_core::{DotallError, PatchedOutput, Result, ValidatedEdit};
use quick_xml::Reader;
use quick_xml::events::Event;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::FORMAT_ID;
use crate::edits::transform::{Axis, AxisChange};
use crate::edits::{EditableValue, XlsxEditOp, parse_validated_operations};

use super::auto_filter;
use super::dimensions;
use super::freeze_panes;
use super::merges;
use super::shared_strings;
use super::structural;
use super::tab_color;
use super::workbook;
use super::worksheet;

pub(super) fn patch(source: &Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
    if edit.format_id != FORMAT_ID {
        return Err(writer_error(format!(
            "cannot apply `{}` edit to an XLSX source",
            edit.format_id
        )));
    }

    let original = fs::read(source).map_err(|error| source_error(source, error))?;
    let operations = parse_validated_operations(&edit.operations)?;
    if let [XlsxEditOp::MergeCells { sheet, range }] = operations.as_slice() {
        return patch_merge(
            &original,
            sheet,
            &merges::MergeEdit::Merge {
                range: range.clone(),
            },
        );
    }
    if let [XlsxEditOp::UnmergeCells { sheet, range }] = operations.as_slice() {
        return patch_merge(
            &original,
            sheet,
            &merges::MergeEdit::Unmerge {
                range: range.clone(),
            },
        );
    }
    if let [
        XlsxEditOp::SetColumnWidth {
            sheet,
            column,
            width,
        },
    ] = operations.as_slice()
    {
        return patch_dimension(
            &original,
            sheet,
            &dimensions::DimensionEdit::ColumnWidth {
                column: column.clone(),
                width: *width,
            },
        );
    }
    if let [XlsxEditOp::SetRowHeight { sheet, row, height }] = operations.as_slice() {
        return patch_dimension(
            &original,
            sheet,
            &dimensions::DimensionEdit::RowHeight {
                row: *row,
                height: *height,
            },
        );
    }
    if let [XlsxEditOp::FreezePanes { sheet, cell }] = operations.as_slice() {
        return patch_freeze_panes(&original, sheet, cell.as_deref());
    }
    if let [XlsxEditOp::SetTabColor { sheet, color }] = operations.as_slice() {
        return patch_tab_color(&original, sheet, color.as_deref());
    }
    if let [XlsxEditOp::SetAutoFilter { sheet, range }] = operations.as_slice() {
        return patch_auto_filter(&original, sheet, range.as_deref());
    }
    if let [XlsxEditOp::DefineName { name, formula }] = operations.as_slice() {
        let patch = workbook::define_name(&original, name, formula)?;
        let bytes = rebuild_package(
            &original,
            &patch.replacements,
            &patch.removals,
            &patch.additions,
        )?;
        return Ok(PatchedOutput {
            after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
        });
    }
    if let [XlsxEditOp::DeleteName { name }] = operations.as_slice() {
        let patch = workbook::delete_name(&original, name)?;
        let bytes = rebuild_package(
            &original,
            &patch.replacements,
            &patch.removals,
            &patch.additions,
        )?;
        return Ok(PatchedOutput {
            after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
        });
    }
    if let [XlsxEditOp::HideSheet { sheet, hidden }] = operations.as_slice() {
        let patch = workbook::hide_sheet(&original, sheet, *hidden)?;
        let bytes = rebuild_package(
            &original,
            &patch.replacements,
            &patch.removals,
            &patch.additions,
        )?;
        return Ok(PatchedOutput {
            after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
        });
    }
    if let [XlsxEditOp::AddSheet { name, after }] = operations.as_slice() {
        let patch = workbook::add_sheet(&original, name, after.as_deref())?;
        let bytes = rebuild_package(
            &original,
            &patch.replacements,
            &patch.removals,
            &patch.additions,
        )?;
        return Ok(PatchedOutput {
            after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
        });
    }
    if let [XlsxEditOp::RenameSheet { from, to }] = operations.as_slice() {
        let patch = workbook::rename_sheet(&original, from, to)?;
        let bytes = rebuild_package(
            &original,
            &patch.replacements,
            &patch.removals,
            &patch.additions,
        )?;
        return Ok(PatchedOutput {
            after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
        });
    }
    if let [XlsxEditOp::DeleteSheet { name, .. }] = operations.as_slice() {
        let patch = workbook::delete_sheet(&original, name)?;
        let bytes = rebuild_package(
            &original,
            &patch.replacements,
            &patch.removals,
            &patch.additions,
        )?;
        return Ok(PatchedOutput {
            after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
        });
    }
    if let [
        operation @ (XlsxEditOp::InsertRow { .. }
        | XlsxEditOp::DeleteRow { .. }
        | XlsxEditOp::InsertColumn { .. }
        | XlsxEditOp::DeleteColumn { .. }),
    ] = operations.as_slice()
    {
        let (edited_sheet, change) = match operation {
            XlsxEditOp::InsertRow { sheet, at, count } => (
                sheet.as_str(),
                AxisChange::Insert {
                    axis: Axis::Row,
                    at: *at,
                    count: *count,
                },
            ),
            XlsxEditOp::DeleteRow { sheet, at, count } => (
                sheet.as_str(),
                AxisChange::Delete {
                    axis: Axis::Row,
                    at: *at,
                    count: *count,
                },
            ),
            XlsxEditOp::InsertColumn { sheet, at, count } => (
                sheet.as_str(),
                AxisChange::Insert {
                    axis: Axis::Column,
                    at: *at,
                    count: *count,
                },
            ),
            XlsxEditOp::DeleteColumn { sheet, at, count } => (
                sheet.as_str(),
                AxisChange::Delete {
                    axis: Axis::Column,
                    at: *at,
                    count: *count,
                },
            ),
            _ => unreachable!(),
        };
        let mut replacements = BTreeMap::new();
        for (sheet, path) in worksheet_paths(&original)? {
            replacements.insert(
                path.clone(),
                structural::patch(
                    &entry_bytes(&original, &path)?,
                    &sheet,
                    edited_sheet,
                    change,
                )?,
            );
        }
        let mut removals = BTreeSet::new();
        invalidate_cached_calculation(&original, &mut replacements, &mut removals)?;
        let bytes = rebuild_package(&original, &replacements, &removals, &BTreeMap::new())?;
        return Ok(PatchedOutput {
            after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
        });
    }
    if operations.iter().any(|operation| {
        matches!(
            operation,
            XlsxEditOp::InsertRow { .. }
                | XlsxEditOp::DeleteRow { .. }
                | XlsxEditOp::InsertColumn { .. }
                | XlsxEditOp::DeleteColumn { .. }
                | XlsxEditOp::AddSheet { .. }
                | XlsxEditOp::RenameSheet { .. }
                | XlsxEditOp::DeleteSheet { .. }
                | XlsxEditOp::SetRange { .. }
                | XlsxEditOp::MergeCells { .. }
                | XlsxEditOp::UnmergeCells { .. }
                | XlsxEditOp::SetColumnWidth { .. }
                | XlsxEditOp::SetRowHeight { .. }
                | XlsxEditOp::FreezePanes { .. }
                | XlsxEditOp::DefineName { .. }
                | XlsxEditOp::DeleteName { .. }
                | XlsxEditOp::HideSheet { .. }
                | XlsxEditOp::SetTabColor { .. }
            | XlsxEditOp::SetAutoFilter { .. }
        )
    }) {
        return Err(DotallError::UnsupportedCapability {
            format_id: FORMAT_ID.into(),
            capability: "structural operation or unexpanded set_range".into(),
            available: vec!["set_cell_value".into(), "set_cell_formula".into()],
        });
    }
    let worksheet_paths = worksheet_paths(&original)?;
    let mut grouped = BTreeMap::<String, Vec<XlsxEditOp>>::new();
    for operation in operations {
        let sheet = match &operation {
            XlsxEditOp::SetCellValue { sheet, .. } | XlsxEditOp::SetCellFormula { sheet, .. } => {
                sheet
            }
            XlsxEditOp::InsertRow { .. }
            | XlsxEditOp::DeleteRow { .. }
            | XlsxEditOp::InsertColumn { .. }
            | XlsxEditOp::DeleteColumn { .. }
            | XlsxEditOp::AddSheet { .. }
            | XlsxEditOp::RenameSheet { .. }
            | XlsxEditOp::DeleteSheet { .. }
            | XlsxEditOp::SetRange { .. }
            | XlsxEditOp::MergeCells { .. }
            | XlsxEditOp::UnmergeCells { .. }
            | XlsxEditOp::SetColumnWidth { .. }
            | XlsxEditOp::SetRowHeight { .. }
            | XlsxEditOp::FreezePanes { .. }
            | XlsxEditOp::DefineName { .. }
            | XlsxEditOp::DeleteName { .. }
            | XlsxEditOp::HideSheet { .. }
            | XlsxEditOp::SetTabColor { .. }
            | XlsxEditOp::SetAutoFilter { .. } => {
                unreachable!("structural operations return above")
            }
        };
        grouped.entry(sheet.clone()).or_default().push(operation);
    }

    let string_values = grouped
        .values()
        .flat_map(|operations| operations.iter())
        .filter_map(|operation| match operation {
            XlsxEditOp::SetCellValue {
                value: EditableValue::String(value),
                ..
            } => Some(value.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut replacements = BTreeMap::new();
    let shared_string_indices = match (string_values.is_empty(), shared_strings_path(&original)?) {
        (false, Some(path)) => {
            let count_delta = grouped
                .iter()
                .try_fold(0_u64, |delta, (sheet, operations)| {
                    let path = worksheet_paths.get(sheet).ok_or_else(|| {
                        writer_error(format!("worksheet path not found for sheet `{sheet}`"))
                    })?;
                    let xml = entry_bytes(&original, path)?;
                    worksheet::shared_string_count_delta(&xml, operations)
                        .map(|value| delta + value)
                })?;
            let patch =
                shared_strings::patch(&entry_bytes(&original, &path)?, string_values, count_delta)?;
            replacements.insert(path, patch.bytes);
            Some(patch.indices)
        }
        _ => None,
    };
    for (sheet, operations) in grouped {
        let path = worksheet_paths
            .get(&sheet)
            .ok_or_else(|| writer_error(format!("worksheet path not found for sheet `{sheet}`")))?;
        let xml = entry_bytes(&original, path)?;
        replacements.insert(
            path.clone(),
            worksheet::patch(&xml, &operations, shared_string_indices.as_ref())?,
        );
    }

    let mut removals = BTreeSet::new();
    invalidate_cached_calculation(&original, &mut replacements, &mut removals)?;
    let bytes = rebuild_package(&original, &replacements, &removals, &BTreeMap::new())?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

/// Drop Excel's calculation chain, strip stale formula/chart caches, and mark
/// the workbook so dependents/charts recalculate on open. Surgical cell patches
/// leave formula `<v>` caches intact; apps that honor those caches (Excel with
/// no force flag, LibreOffice) keep showing the pre-edit graph and KPIs.
fn invalidate_cached_calculation(
    package: &[u8],
    replacements: &mut BTreeMap<String, Vec<u8>>,
    removals: &mut BTreeSet<String>,
) -> Result<()> {
    if has_entry(package, "xl/calcChain.xml")? {
        removals.insert("xl/calcChain.xml".to_owned());
        replacements.insert(
            "[Content_Types].xml".into(),
            remove_calc_chain_override(&entry_bytes(package, "[Content_Types].xml")?)?,
        );
        replacements.insert(
            "xl/_rels/workbook.xml.rels".into(),
            remove_calc_chain_relationship(&entry_bytes(package, "xl/_rels/workbook.xml.rels")?)?,
        );
    }

    for path in worksheet_paths(package)?.into_values() {
        let current = match replacements.get(&path) {
            Some(bytes) => bytes.clone(),
            None => entry_bytes(package, &path)?,
        };
        if let Some(stripped) = strip_formula_cached_values(&current)? {
            replacements.insert(path, stripped);
        }
    }

    for path in chart_paths(package)? {
        let current = match replacements.get(&path) {
            Some(bytes) => bytes.clone(),
            None => entry_bytes(package, &path)?,
        };
        if let Some(cleared) = clear_chart_number_caches(&current)? {
            replacements.insert(path, cleared);
        }
    }

    if let Some(workbook) = ensure_full_calc_on_load(&entry_bytes(package, "xl/workbook.xml")?)? {
        replacements.insert("xl/workbook.xml".into(), workbook);
    }
    Ok(())
}

fn ensure_full_calc_on_load(xml: &[u8]) -> Result<Option<Vec<u8>>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    if let Some(start) = source.find("<calcPr") {
        let end = source[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error("unterminated workbook calcPr"))?;
        let tag = &source[start..end];
        // Workbooks we generate already have fullCalcOnLoad="1"; still need
        // forceFullCalc so Excel rebuilds dependents after surgical value edits.
        if tag.contains(r#"fullCalcOnLoad="1""#) && tag.contains(r#"forceFullCalc="1""#) {
            return Ok(None);
        }
        let updated = rewrite_calc_pr_tag(tag)?;
        let mut output = String::with_capacity(source.len() + 32);
        output.push_str(&source[..start]);
        output.push_str(&updated);
        output.push_str(&source[end..]);
        return Ok(Some(output.into_bytes()));
    }
    let insertion = source
        .rfind("</workbook>")
        .ok_or_else(|| writer_error("workbook XML is missing </workbook>"))?;
    let mut output = String::with_capacity(source.len() + 64);
    output.push_str(&source[..insertion]);
    output.push_str(r#"<calcPr fullCalcOnLoad="1" forceFullCalc="1"/>"#);
    output.push_str(&source[insertion..]);
    Ok(Some(output.into_bytes()))
}

/// Remove cached `<v>` results from formula cells so spreadsheet apps must
/// recompute instead of displaying the pre-edit values.
fn strip_formula_cached_values(xml: &[u8]) -> Result<Option<Vec<u8>>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    let mut changed = false;
    while let Some(relative) = find_cell_start(&source[cursor..]) {
        let start = cursor + relative;
        output.push_str(&source[cursor..start]);
        let end = find_cell_end(&source[start..])?;
        let cell = &source[start..start + end];
        if cell_has_formula(cell) {
            let stripped = remove_cell_value_elements(cell);
            if stripped != cell {
                changed = true;
            }
            output.push_str(&stripped);
        } else {
            output.push_str(cell);
        }
        cursor = start + end;
    }
    output.push_str(&source[cursor..]);
    if changed {
        Ok(Some(output.into_bytes()))
    } else {
        Ok(None)
    }
}

fn find_cell_start(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index] == b'<' && bytes[index + 1] == b'c' {
            match bytes[index + 2] {
                b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/' => return Some(index),
                _ => {}
            }
        }
        index += 1;
    }
    None
}

fn find_cell_end(cell_start: &str) -> Result<usize> {
    if let Some(relative) = cell_start.find("/>") {
        let close = cell_start.find("</c>");
        if close.is_none_or(|close| relative < close) {
            return Ok(relative + 2);
        }
    }
    cell_start
        .find("</c>")
        .map(|offset| offset + 4)
        .ok_or_else(|| writer_error("unterminated worksheet cell"))
}

fn cell_has_formula(cell: &str) -> bool {
    cell.contains("<f>") || cell.contains("<f ") || cell.contains("<f/")
}

fn remove_cell_value_elements(cell: &str) -> String {
    let mut output = String::with_capacity(cell.len());
    let mut cursor = 0;
    while let Some(relative) = cell[cursor..].find("<v") {
        let start = cursor + relative;
        let after = &cell[start + 2..];
        let is_value = after
            .chars()
            .next()
            .is_some_and(|character| matches!(character, '>' | '/' | ' ' | '\t' | '\n' | '\r'));
        if !is_value {
            output.push_str(&cell[cursor..start + 2]);
            cursor = start + 2;
            continue;
        }
        output.push_str(&cell[cursor..start]);
        if let Some(end) = cell[start..].find("</v>") {
            cursor = start + end + 4;
        } else if let Some(end) = cell[start..].find("/>") {
            cursor = start + end + 2;
        } else {
            output.push_str(&cell[start..]);
            return output;
        }
    }
    output.push_str(&cell[cursor..]);
    output
}

/// Drop cached chart series points so charts refresh from recalculated cells.
fn clear_chart_number_caches(xml: &[u8]) -> Result<Option<Vec<u8>>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("chart XML is not UTF-8: {error}")))?;
    if !source.contains("<c:pt") {
        return Ok(None);
    }
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    let mut changed = false;
    while let Some((relative, close)) = next_chart_cache(&source[cursor..]) {
        let start = cursor + relative;
        output.push_str(&source[cursor..start]);
        let tag_end = source[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error("unterminated chart cache tag"))?;
        let tag = &source[start..tag_end];
        let end = source[tag_end..]
            .find(close)
            .map(|offset| tag_end + offset + close.len())
            .ok_or_else(|| writer_error(format!("unterminated chart cache `{close}`")))?;
        let body = &source[tag_end..end - close.len()];
        output.push_str(tag);
        if let Some(format) = extract_chart_format_code(body) {
            output.push_str(&format);
        }
        let count = extract_chart_pt_count(body).unwrap_or(0);
        output.push_str(&format!(r#"<c:ptCount val="{count}"/>"#));
        output.push_str(close);
        if body.contains("<c:pt") {
            changed = true;
        }
        cursor = end;
    }
    output.push_str(&source[cursor..]);
    if changed {
        Ok(Some(output.into_bytes()))
    } else {
        Ok(None)
    }
}

fn next_chart_cache(source: &str) -> Option<(usize, &'static str)> {
    let num = source
        .find("<c:numCache>")
        .map(|offset| (offset, "</c:numCache>"));
    let str_cache = source
        .find("<c:strCache>")
        .map(|offset| (offset, "</c:strCache>"));
    match (num, str_cache) {
        (Some(num), Some(str_cache)) if str_cache.0 < num.0 => Some(str_cache),
        (Some(num), _) => Some(num),
        (None, Some(str_cache)) => Some(str_cache),
        (None, None) => None,
    }
}

fn extract_chart_format_code(body: &str) -> Option<String> {
    let start = body.find("<c:formatCode>")?;
    let end = body[start..].find("</c:formatCode>")? + start + "</c:formatCode>".len();
    Some(body[start..end].to_owned())
}

fn extract_chart_pt_count(body: &str) -> Option<u32> {
    let start = body.find("<c:ptCount ")?;
    let end = body[start..].find('>').map(|offset| start + offset + 1)?;
    let tag = &body[start..end];
    let value_start = tag.find(r#"val=""#)? + 5;
    let value_end = tag[value_start..].find('"')? + value_start;
    tag[value_start..value_end].parse().ok()
}

fn chart_paths(package: &[u8]) -> Result<Vec<String>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| writer_error(format!("invalid XLSX package: {error}")))?;
    let mut paths = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| writer_error(format!("cannot read ZIP entry: {error}")))?;
        let name = entry.name();
        if name.starts_with("xl/charts/") && name.ends_with(".xml") {
            paths.push(name.to_owned());
        }
    }
    Ok(paths)
}

fn rewrite_calc_pr_tag(tag: &str) -> Result<String> {
    let self_closing = tag.ends_with("/>");
    let inner = tag
        .trim_start_matches("<calcPr")
        .trim_end_matches("/>")
        .trim_end_matches('>')
        .trim();
    let mut attributes = Vec::new();
    let mut saw_full_calc = false;
    let mut saw_force_full = false;
    for attribute in inner.split_whitespace() {
        if attribute.starts_with("fullCalcOnLoad=") {
            attributes.push(r#"fullCalcOnLoad="1""#.to_owned());
            saw_full_calc = true;
        } else if attribute.starts_with("forceFullCalc=") {
            attributes.push(r#"forceFullCalc="1""#.to_owned());
            saw_force_full = true;
        } else {
            attributes.push(attribute.to_owned());
        }
    }
    if !saw_full_calc {
        attributes.push(r#"fullCalcOnLoad="1""#.to_owned());
    }
    if !saw_force_full {
        attributes.push(r#"forceFullCalc="1""#.to_owned());
    }
    let body = attributes.join(" ");
    if self_closing {
        Ok(format!("<calcPr {body}/>"))
    } else {
        Ok(format!("<calcPr {body}>"))
    }
}

fn patch_merge(original: &[u8], sheet: &str, edit: &merges::MergeEdit) -> Result<PatchedOutput> {
    let worksheet_paths = worksheet_paths(original)?;
    let path = worksheet_paths
        .get(sheet)
        .ok_or_else(|| writer_error(format!("worksheet path not found for sheet `{sheet}`")))?;
    let xml = entry_bytes(original, path)?;
    let mut replacements = BTreeMap::new();
    replacements.insert(path.clone(), merges::patch(&xml, edit)?);
    let bytes = rebuild_package(original, &replacements, &BTreeSet::new(), &BTreeMap::new())?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn patch_dimension(
    original: &[u8],
    sheet: &str,
    edit: &dimensions::DimensionEdit,
) -> Result<PatchedOutput> {
    let worksheet_paths = worksheet_paths(original)?;
    let path = worksheet_paths
        .get(sheet)
        .ok_or_else(|| writer_error(format!("worksheet path not found for sheet `{sheet}`")))?;
    let xml = entry_bytes(original, path)?;
    let mut replacements = BTreeMap::new();
    replacements.insert(path.clone(), dimensions::patch(&xml, edit)?);
    let bytes = rebuild_package(original, &replacements, &BTreeSet::new(), &BTreeMap::new())?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn patch_freeze_panes(original: &[u8], sheet: &str, cell: Option<&str>) -> Result<PatchedOutput> {
    let worksheet_paths = worksheet_paths(original)?;
    let path = worksheet_paths
        .get(sheet)
        .ok_or_else(|| writer_error(format!("worksheet path not found for sheet `{sheet}`")))?;
    let xml = entry_bytes(original, path)?;
    let mut replacements = BTreeMap::new();
    replacements.insert(path.clone(), freeze_panes::patch(&xml, cell)?);
    let bytes = rebuild_package(original, &replacements, &BTreeSet::new(), &BTreeMap::new())?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn patch_tab_color(original: &[u8], sheet: &str, color: Option<&str>) -> Result<PatchedOutput> {
    let worksheet_paths = worksheet_paths(original)?;
    let path = worksheet_paths
        .get(sheet)
        .ok_or_else(|| writer_error(format!("worksheet path not found for sheet `{sheet}`")))?;
    let xml = entry_bytes(original, path)?;
    let mut replacements = BTreeMap::new();
    replacements.insert(path.clone(), tab_color::patch(&xml, color)?);
    let bytes = rebuild_package(original, &replacements, &BTreeSet::new(), &BTreeMap::new())?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn patch_auto_filter(original: &[u8], sheet: &str, range: Option<&str>) -> Result<PatchedOutput> {
    let worksheet_paths = worksheet_paths(original)?;
    let path = worksheet_paths
        .get(sheet)
        .ok_or_else(|| writer_error(format!("worksheet path not found for sheet `{sheet}`")))?;
    let xml = entry_bytes(original, path)?;
    let mut replacements = BTreeMap::new();
    replacements.insert(path.clone(), auto_filter::patch(&xml, range)?);
    let bytes = rebuild_package(original, &replacements, &BTreeSet::new(), &BTreeMap::new())?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn shared_strings_path(package: &[u8]) -> Result<Option<String>> {
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels")?;
    if let Some(target) = parse_shared_strings_target(&relationships)? {
        return Ok(Some(normalize_relationship_target(&target)));
    }
    Ok(has_entry(package, "xl/sharedStrings.xml")?.then(|| "xl/sharedStrings.xml".into()))
}

fn parse_shared_strings_target(xml: &[u8]) -> Result<Option<String>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| writer_error(format!("invalid workbook relationships XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if element.name().as_ref() == b"Relationship" =>
            {
                let mut relationship_type = None;
                let mut target = None;
                for attribute in element.attributes().flatten() {
                    match attribute.key.as_ref() {
                        b"Type" => {
                            relationship_type =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        b"Target" => {
                            target =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        _ => {}
                    }
                }
                if relationship_type
                    .as_deref()
                    .is_some_and(|value| value.ends_with("/sharedStrings"))
                {
                    return target.map(Some).ok_or_else(|| {
                        writer_error("shared strings relationship is missing its target")
                    });
                }
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
        buffer.clear();
    }
}

fn worksheet_paths(package: &[u8]) -> Result<BTreeMap<String, String>> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels")?;
    let relationship_targets = parse_relationships(&relationships)?;
    parse_sheets(&workbook)?
        .into_iter()
        .map(|(sheet, relationship_id)| {
            let target = relationship_targets.get(&relationship_id).ok_or_else(|| {
                writer_error(format!(
                    "workbook relationship `{relationship_id}` not found for sheet `{sheet}`"
                ))
            })?;
            Ok((sheet, normalize_relationship_target(target)))
        })
        .collect()
}

fn parse_sheets(xml: &[u8]) -> Result<Vec<(String, String)>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut sheets = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| writer_error(format!("invalid workbook XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if element.name().as_ref() == b"sheet" =>
            {
                let mut name = None;
                let mut relationship_id = None;
                for attribute in element.attributes().flatten() {
                    match attribute.key.as_ref() {
                        b"name" => {
                            name = Some(
                                quick_xml::escape::unescape(&String::from_utf8_lossy(
                                    attribute.value.as_ref(),
                                ))
                                .map_err(|error| {
                                    writer_error(format!("invalid worksheet name: {error}"))
                                })?
                                .into_owned(),
                            )
                        }
                        b"r:id" => {
                            relationship_id =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        _ => {}
                    }
                }
                if let (Some(name), Some(relationship_id)) = (name, relationship_id) {
                    sheets.push((name, relationship_id));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(sheets)
}

fn parse_relationships(xml: &[u8]) -> Result<BTreeMap<String, String>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut relationships = BTreeMap::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| writer_error(format!("invalid workbook relationships XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if element.name().as_ref() == b"Relationship" =>
            {
                let mut id = None;
                let mut target = None;
                for attribute in element.attributes().flatten() {
                    match attribute.key.as_ref() {
                        b"Id" => {
                            id =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        b"Target" => {
                            target =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        _ => {}
                    }
                }
                if let (Some(id), Some(target)) = (id, target) {
                    relationships.insert(id, target);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(relationships)
}

fn normalize_relationship_target(target: &str) -> String {
    if target.starts_with('/') {
        target.trim_start_matches('/').to_owned()
    } else {
        format!("xl/{target}")
    }
}

fn has_entry(package: &[u8], name: &str) -> Result<bool> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| writer_error(format!("invalid XLSX package: {error}")))?;
    Ok(archive.by_name(name).is_ok())
}

fn entry_bytes(package: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| writer_error(format!("invalid XLSX package: {error}")))?;
    let mut entry = archive
        .by_name(name)
        .map_err(|error| writer_error(format!("XLSX package is missing `{name}`: {error}")))?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| writer_error(format!("cannot read `{name}`: {error}")))?;
    Ok(bytes)
}

fn rebuild_package(
    original: &[u8],
    replacements: &BTreeMap<String, Vec<u8>>,
    removals: &BTreeSet<String>,
    additions: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(original))
        .map_err(|error| writer_error(format!("invalid XLSX package: {error}")))?;
    let output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(output);

    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| writer_error(format!("cannot read ZIP entry: {error}")))?;
        let name = entry.name().to_owned();
        if removals.contains(&name) {
            continue;
        }
        if let Some(replacement) = replacements.get(&name) {
            let options = SimpleFileOptions::default()
                .compression_method(entry.compression())
                .last_modified_time(entry.last_modified().unwrap_or_default());
            writer.start_file(name, options).map_err(|error| {
                writer_error(format!("cannot start patched ZIP entry: {error}"))
            })?;
            writer.write_all(replacement).map_err(|error| {
                writer_error(format!("cannot write patched ZIP entry: {error}"))
            })?;
        } else {
            writer
                .raw_copy_file(entry)
                .map_err(|error| writer_error(format!("cannot copy ZIP entry: {error}")))?;
        }
    }
    for (name, bytes) in additions {
        writer
            .start_file(name, SimpleFileOptions::default())
            .map_err(|error| writer_error(format!("cannot start added ZIP entry: {error}")))?;
        writer
            .write_all(bytes)
            .map_err(|error| writer_error(format!("cannot write added ZIP entry: {error}")))?;
    }

    writer
        .finish()
        .map_err(|error| writer_error(format!("cannot finish XLSX package: {error}")))
        .map(|cursor| cursor.into_inner())
}

fn remove_calc_chain_override(xml: &[u8]) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("content types XML is not UTF-8: {error}")))?;
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("<Override") {
        let start = cursor + relative;
        output.push_str(&source[cursor..start]);
        let end = source[start..]
            .find('>')
            .map(|offset| start + offset)
            .ok_or_else(|| writer_error("unterminated content types Override"))?;
        let tag = &source[start..=end];
        if !tag.contains(r#"PartName="/xl/calcChain.xml""#) {
            output.push_str(tag);
        }
        cursor = end + 1;
    }
    output.push_str(&source[cursor..]);
    Ok(output.into_bytes())
}

fn remove_calc_chain_relationship(xml: &[u8]) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml).map_err(|error| {
        writer_error(format!("workbook relationships XML is not UTF-8: {error}"))
    })?;
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("<Relationship") {
        let start = cursor + relative;
        output.push_str(&source[cursor..start]);
        let end = source[start..]
            .find('>')
            .map(|offset| start + offset)
            .ok_or_else(|| writer_error("unterminated workbook Relationship"))?;
        let tag = &source[start..=end];
        if !tag.contains(r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain""#)
        {
            output.push_str(tag);
        }
        cursor = end + 1;
    }
    output.push_str(&source[cursor..]);
    Ok(output.into_bytes())
}

fn source_error(source: &Path, error: std::io::Error) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: source.into(),
        message: error.to_string(),
    }
}

fn writer_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx package writer>".into(),
        message: message.into(),
    }
}
