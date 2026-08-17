use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use dotall_core::{DotallError, Result};
use quick_xml::Reader;
use quick_xml::events::Event;
use zip::ZipArchive;

use crate::FORMAT_ID;
use crate::dependencies::reference_spans;

pub(super) struct PackagePatch {
    pub replacements: BTreeMap<String, Vec<u8>>,
    pub additions: BTreeMap<String, Vec<u8>>,
    pub removals: BTreeSet<String>,
}

pub(super) fn add_sheet(package: &[u8], name: &str, after: Option<&str>) -> Result<PackagePatch> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels")?;
    let content_types = entry_bytes(package, "[Content_Types].xml")?;
    let sheets = sheets(&workbook)?;
    let relationship_ids = relationship_ids(&relationships)?;
    let part_numbers = worksheet_part_numbers(package)?;
    let sheet_ids = sheets.iter().map(|sheet| sheet.sheet_id).collect();
    let sheet_id = lowest_unused_number(&sheet_ids);
    let relationship_id = lowest_unused(&relationship_ids, "rId");
    let part_number = lowest_unused(&part_numbers, "");
    let target = format!("worksheets/sheet{part_number}.xml");
    let insertion_index = after
        .map(|after| {
            sheets
                .iter()
                .position(|sheet| sheet.name.eq_ignore_ascii_case(after))
                .map(|index| index + 1)
                .ok_or_else(|| writer_error(format!("worksheet `{after}` was not found")))
        })
        .transpose()?
        .unwrap_or(sheets.len());

    let mut replacements = BTreeMap::new();
    replacements.insert(
        "xl/workbook.xml".into(),
        insert_sheet(
            &workbook,
            &format!(
                r#"<sheet name="{}" sheetId="{sheet_id}" r:id="{relationship_id}"/>"#,
                escape_xml(name)
            ),
            &sheets,
            insertion_index,
        )?,
    );
    replacements.insert(
        "xl/_rels/workbook.xml.rels".into(),
        insert_before_close(
            &relationships,
            "Relationships",
            &format!(
                r#"<Relationship Id="{relationship_id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="{target}"/>"#
            ),
        )?,
    );
    replacements.insert(
        "[Content_Types].xml".into(),
        insert_before_close(
            &content_types,
            "Types",
            &format!(
                r#"<Override PartName="/xl/worksheets/sheet{part_number}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
            ),
        )?,
    );
    if has_entry(package, "docProps/app.xml")? {
        replacements.insert(
            "docProps/app.xml".into(),
            add_app_sheet_title(&entry_bytes(package, "docProps/app.xml")?, name)?,
        );
    }

    Ok(PackagePatch {
        replacements,
        additions: BTreeMap::from([(
            format!("xl/worksheets/sheet{part_number}.xml"),
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec(),
        )]),
        removals: BTreeSet::new(),
    })
}

/// Create or update a workbook-scoped defined name. Patches only `xl/workbook.xml`.
pub(super) fn define_name(package: &[u8], name: &str, formula: &str) -> Result<PackagePatch> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let patched = upsert_defined_name(&workbook, name, formula)?;
    Ok(PackagePatch {
        replacements: BTreeMap::from([("xl/workbook.xml".into(), patched)]),
        additions: BTreeMap::new(),
        removals: BTreeSet::new(),
    })
}

/// Set or clear sheet print area (`_xlnm.Print_Area` with localSheetId). Patches only workbook.xml.
pub(super) fn set_print_area(
    package: &[u8],
    sheet_name: &str,
    range: Option<&str>,
) -> Result<PackagePatch> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let sheets = sheets(&workbook)?;
    let sheet = find_sheet(&sheets, sheet_name)?;
    let local_sheet_id = sheets
        .iter()
        .position(|candidate| candidate.name.eq_ignore_ascii_case(&sheet.name))
        .ok_or_else(|| writer_error(format!("worksheet `{}` was not found", sheet.name)))?;
    let patched = match range {
        Some(a1) => {
            let formula = print_area_formula(&sheet.name, a1);
            upsert_local_defined_name(&workbook, "_xlnm.Print_Area", local_sheet_id, &formula)?
        }
        None => remove_local_defined_name(&workbook, "_xlnm.Print_Area", local_sheet_id)?,
    };
    Ok(PackagePatch {
        replacements: BTreeMap::from([("xl/workbook.xml".into(), patched)]),
        additions: BTreeMap::new(),
        removals: BTreeSet::new(),
    })
}

/// Set or clear sheet print titles (`_xlnm.Print_Titles`). Patches only workbook.xml.
pub(super) fn set_print_titles(
    package: &[u8],
    sheet_name: &str,
    rows: Option<&str>,
    cols: Option<&str>,
) -> Result<PackagePatch> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let sheets = sheets(&workbook)?;
    let sheet = find_sheet(&sheets, sheet_name)?;
    let local_sheet_id = sheets
        .iter()
        .position(|candidate| candidate.name.eq_ignore_ascii_case(&sheet.name))
        .ok_or_else(|| writer_error(format!("worksheet `{}` was not found", sheet.name)))?;
    let patched = match (rows, cols) {
        (None, None) => remove_local_defined_name(&workbook, "_xlnm.Print_Titles", local_sheet_id)?,
        _ => {
            let formula = print_titles_formula(&sheet.name, rows, cols);
            upsert_local_defined_name(&workbook, "_xlnm.Print_Titles", local_sheet_id, &formula)?
        }
    };
    Ok(PackagePatch {
        replacements: BTreeMap::from([("xl/workbook.xml".into(), patched)]),
        additions: BTreeMap::new(),
        removals: BTreeSet::new(),
    })
}

fn print_area_formula(sheet: &str, a1: &str) -> String {
    let absolute = a1_to_absolute(a1);
    if sheet_needs_quotes(sheet) {
        format!("'{}'!{absolute}", sheet.replace('\'', "''"))
    } else {
        format!("{sheet}!{absolute}")
    }
}

fn print_titles_formula(sheet: &str, rows: Option<&str>, cols: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(rows) = rows {
        let absolute = rows
            .split(':')
            .map(|part| format!("${part}"))
            .collect::<Vec<_>>()
            .join(":");
        parts.push(if sheet_needs_quotes(sheet) {
            format!("'{}'!{absolute}", sheet.replace('\'', "''"))
        } else {
            format!("{sheet}!{absolute}")
        });
    }
    if let Some(cols) = cols {
        let absolute = cols
            .split(':')
            .map(|part| format!("${part}"))
            .collect::<Vec<_>>()
            .join(":");
        parts.push(if sheet_needs_quotes(sheet) {
            format!("'{}'!{absolute}", sheet.replace('\'', "''"))
        } else {
            format!("{sheet}!{absolute}")
        });
    }
    parts.join(",")
}

fn a1_to_absolute(a1: &str) -> String {
    // A1:B3 -> $A$1:$B$3
    a1.split(':')
        .map(|part| {
            let split = part
                .find(|c: char| c.is_ascii_digit())
                .unwrap_or(part.len());
            let (col, row) = part.split_at(split);
            format!("${col}${row}")
        })
        .collect::<Vec<_>>()
        .join(":")
}

fn sheet_needs_quotes(name: &str) -> bool {
    name.chars()
        .any(|c| !(c.is_ascii_alphanumeric() || c == '_'))
        || name.is_empty()
}

fn upsert_local_defined_name(
    xml: &[u8],
    defined_name: &str,
    local_sheet_id: usize,
    formula: &str,
) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    let escaped_formula = escape_xml(formula);
    let tag = format!(
        r#"<definedName name="{defined_name}" localSheetId="{local_sheet_id}">{escaped_formula}</definedName>"#
    );
    if let Some((open_start, _open_end, close_end)) =
        find_local_defined_name(text, defined_name, local_sheet_id)?
    {
        return Ok(format!("{}{}{}", &text[..open_start], tag, &text[close_end..]).into_bytes());
    }
    if let Some(container_end) = find_defined_names_close(text)? {
        return Ok(format!(
            "{}{}{}",
            &text[..container_end],
            tag,
            &text[container_end..]
        )
        .into_bytes());
    }
    let sheets_close = text
        .find("</sheets>")
        .map(|offset| offset + "</sheets>".len())
        .ok_or_else(|| writer_error("workbook XML is missing </sheets>"))?;
    Ok(format!(
        "{}<definedNames>{}</definedNames>{}",
        &text[..sheets_close],
        tag,
        &text[sheets_close..]
    )
    .into_bytes())
}

fn remove_local_defined_name(
    xml: &[u8],
    defined_name: &str,
    local_sheet_id: usize,
) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    let Some((open_start, _open_end, close_end)) =
        find_local_defined_name(text, defined_name, local_sheet_id)?
    else {
        return Ok(xml.to_vec());
    };
    let mut patched = format!("{}{}", &text[..open_start], &text[close_end..]);
    if let Some(start) = patched.find("<definedNames") {
        let open_end = patched[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error("unterminated definedNames"))?;
        if !patched[start..open_end].ends_with("/>") {
            let close = patched[open_end..]
                .find("</definedNames>")
                .map(|offset| open_end + offset)
                .ok_or_else(|| writer_error("unterminated definedNames"))?;
            let close_end = close + "</definedNames>".len();
            let inner = patched[open_end..close].trim();
            if inner.is_empty() {
                patched = format!("{}{}", &patched[..start], &patched[close_end..]);
            }
        }
    }
    Ok(patched.into_bytes())
}

fn find_local_defined_name(
    text: &str,
    defined_name: &str,
    local_sheet_id: usize,
) -> Result<Option<(usize, usize, usize)>> {
    let mut cursor = 0;
    let needle = format!(r#"name="{defined_name}""#);
    let local_needle = format!(r#"localSheetId="{local_sheet_id}""#);
    while let Some(offset) = text[cursor..].find("<definedName") {
        let start = cursor + offset;
        if text[start + "<definedName".len()..].starts_with('s') {
            cursor = start + "<definedName".len();
            continue;
        }
        let open_end = text[start..]
            .find('>')
            .map(|rel| start + rel + 1)
            .ok_or_else(|| writer_error("unterminated definedName"))?;
        let open_tag = &text[start..open_end];
        let close = text[open_end..]
            .find("</definedName>")
            .map(|rel| open_end + rel)
            .ok_or_else(|| writer_error("unterminated definedName"))?;
        let close_end = close + "</definedName>".len();
        if open_tag.contains(&needle) && open_tag.contains(&local_needle) {
            return Ok(Some((start, open_end, close_end)));
        }
        cursor = close_end;
    }
    Ok(None)
}

/// Remove a workbook-scoped defined name. Patches only `xl/workbook.xml`.
pub(super) fn delete_name(package: &[u8], name: &str) -> Result<PackagePatch> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let patched = remove_defined_name(&workbook, name)?;
    Ok(PackagePatch {
        replacements: BTreeMap::from([("xl/workbook.xml".into(), patched)]),
        additions: BTreeMap::new(),
        removals: BTreeSet::new(),
    })
}

/// Set or clear workbook sheet `state="hidden"`. Patches only `xl/workbook.xml`.
pub(super) fn hide_sheet(package: &[u8], name: &str, hidden: bool) -> Result<PackagePatch> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let sheets = sheets(&workbook)?;
    let sheet = find_sheet(&sheets, name)?;
    if hidden {
        let visible_count = sheets.iter().filter(|sheet| sheet.visible).count();
        if sheet.visible && visible_count <= 1 {
            return Err(writer_error("cannot hide the last visible worksheet"));
        }
    }
    let patched = set_sheet_hidden_state(&workbook, sheet.tag_start, sheet.tag_end, hidden)?;
    Ok(PackagePatch {
        replacements: BTreeMap::from([("xl/workbook.xml".into(), patched)]),
        additions: BTreeMap::new(),
        removals: BTreeSet::new(),
    })
}

/// Returns `(sheet_name, is_hidden)` for each workbook sheet.
pub(super) fn sheet_visibility(package: &[u8]) -> Result<Vec<(String, bool)>> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    Ok(sheets(&workbook)?
        .into_iter()
        .map(|sheet| (sheet.name, !sheet.visible))
        .collect())
}

fn set_sheet_hidden_state(
    xml: &[u8],
    tag_start: usize,
    tag_end: usize,
    hidden: bool,
) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    let tag = &text[tag_start..tag_end];
    let without_state = remove_attribute(tag, "state")?;
    let replacement = if hidden {
        insert_attribute_before_close(&without_state, r#" state="hidden""#)?
    } else {
        without_state
    };
    Ok(format!("{}{}{}", &text[..tag_start], replacement, &text[tag_end..]).into_bytes())
}

fn remove_attribute(tag: &str, attribute: &str) -> Result<String> {
    let needle = format!("{attribute}=\"");
    let Some(rel) = tag.find(&needle) else {
        return Ok(tag.to_owned());
    };
    let value_start = rel + needle.len();
    let value_end = tag[value_start..]
        .find('"')
        .map(|offset| value_start + offset)
        .ok_or_else(|| writer_error(format!("unterminated `{attribute}` attribute")))?;
    let mut start = rel;
    while start > 0 && tag.as_bytes()[start - 1] == b' ' {
        start -= 1;
    }
    Ok(format!("{}{}", &tag[..start], &tag[value_end + 1..]))
}

fn insert_attribute_before_close(tag: &str, attribute: &str) -> Result<String> {
    if tag.ends_with("/>") {
        let open = tag.trim_end_matches("/>").trim_end();
        return Ok(format!("{open}{attribute}/>"));
    }
    if tag.ends_with('>') {
        let open = tag.trim_end_matches('>').trim_end();
        return Ok(format!("{open}{attribute}>"));
    }
    Err(writer_error("unterminated workbook sheet tag"))
}

pub(super) fn rename_sheet(package: &[u8], from: &str, to: &str) -> Result<PackagePatch> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let sheets = sheets(&workbook)?;
    let sheet = sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(from))
        .ok_or_else(|| writer_error(format!("worksheet `{from}` was not found")))?;
    let mut replacements = BTreeMap::new();
    replacements.insert(
        "xl/workbook.xml".into(),
        replace_tag_attribute(&workbook, sheet.tag_start, sheet.tag_end, "name", to)?,
    );
    for path in worksheet_paths(package, &workbook)? {
        let xml = entry_bytes(package, &path)?;
        let patched = rewrite_worksheet_formulas(&xml, from, to)?;
        if patched != xml {
            replacements.insert(path, patched);
        }
    }
    if has_entry(package, "docProps/app.xml")? {
        replacements.insert(
            "docProps/app.xml".into(),
            rename_app_sheet_title(&entry_bytes(package, "docProps/app.xml")?, from, to)?,
        );
    }
    Ok(PackagePatch {
        replacements,
        additions: BTreeMap::new(),
        removals: BTreeSet::new(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteSheetReferences {
    pub sheet_name: String,
    pub formula_cells: Vec<String>,
    pub defined_names: Vec<String>,
}

pub(crate) fn delete_sheet_references(package: &[u8], name: &str) -> Result<DeleteSheetReferences> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let sheets = sheets(&workbook)?;
    let sheet = find_sheet(&sheets, name)?;
    if sheets.iter().filter(|sheet| sheet.visible).count() <= 1 && sheet.visible {
        return Err(writer_error("cannot delete the last visible sheet"));
    }
    let mut formula_cells = Vec::new();
    for (sheet_name, path) in worksheet_paths_with_names(package, &workbook)? {
        for address in formula_cells_referencing_sheet(&entry_bytes(package, &path)?, &sheet.name)?
        {
            formula_cells.push(format!("{sheet_name}!{address}"));
        }
    }
    let defined_names = defined_names_referencing_sheet(&workbook, &sheet.name)?;
    Ok(DeleteSheetReferences {
        sheet_name: sheet.name.clone(),
        formula_cells,
        defined_names,
    })
}

pub(super) fn delete_sheet(package: &[u8], name: &str) -> Result<PackagePatch> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels")?;
    let content_types = entry_bytes(package, "[Content_Types].xml")?;
    let sheets = sheets(&workbook)?;
    let sheet = find_sheet(&sheets, name)?;
    if sheets.iter().filter(|sheet| sheet.visible).count() <= 1 && sheet.visible {
        return Err(writer_error("cannot delete the last visible sheet"));
    }
    let worksheet = relationship_targets(&relationships)?
        .get(&sheet.relationship_id)
        .map(|target| normalize_target(target))
        .ok_or_else(|| {
            writer_error(format!(
                "workbook relationship `{}` is missing",
                sheet.relationship_id
            ))
        })?;
    let worksheet_relationships = relationship_part_name(&worksheet);
    let mut removals = BTreeSet::from([worksheet.clone(), worksheet_relationships.clone()]);
    removals.extend(orphaned_visual_parts(package, &worksheet_relationships)?);
    let mut replacements = BTreeMap::new();
    replacements.insert(
        "xl/workbook.xml".into(),
        rewrite_defined_name_formulas(
            &remove_span(&workbook, sheet.tag_start, sheet.tag_end)?,
            &sheet.name,
        )?,
    );
    replacements.insert(
        "xl/_rels/workbook.xml.rels".into(),
        remove_relationship(&relationships, &sheet.relationship_id)?,
    );
    replacements.insert(
        "[Content_Types].xml".into(),
        remove_content_type_overrides(&content_types, &removals)?,
    );
    if has_entry(package, "docProps/app.xml")? {
        replacements.insert(
            "docProps/app.xml".into(),
            remove_app_sheet_title(&entry_bytes(package, "docProps/app.xml")?, &sheet.name)?,
        );
    }
    for (sheet_name, path) in worksheet_paths_with_names(package, &workbook)? {
        if sheet_name.eq_ignore_ascii_case(&sheet.name) {
            continue;
        }
        let xml = entry_bytes(package, &path)?;
        let patched = rewrite_worksheet_formulas(&xml, &sheet.name, "#REF!")?;
        if patched != xml {
            replacements.insert(path, patched);
        }
    }

    Ok(PackagePatch {
        replacements,
        additions: BTreeMap::new(),
        removals,
    })
}

pub(crate) fn validate_rename_safety(package: &[u8], from: &str) -> Result<()> {
    let references = [
        sheet_reference_prefix(from),
        format!("'{}'!", from.replace('\'', "''")),
    ];
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| writer_error(format!("invalid XLSX package: {error}")))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| writer_error(format!("cannot read ZIP entry: {error}")))?;
        let path = entry.name().to_owned();
        if path.starts_with("xl/worksheets/") || !path.ends_with(".xml") {
            continue;
        }
        let mut xml = String::new();
        entry
            .read_to_string(&mut xml)
            .map_err(|error| writer_error(format!("cannot read `{path}`: {error}")))?;
        if references.iter().any(|reference| xml.contains(reference)) {
            return Err(writer_error(format!(
                "rename_sheet is unsafe: `{path}` contains references to `{from}` that this writer does not rewrite"
            )));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct Sheet {
    name: String,
    sheet_id: u32,
    relationship_id: String,
    visible: bool,
    tag_start: usize,
    tag_end: usize,
}

fn sheets(xml: &[u8]) -> Result<Vec<Sheet>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    let mut sheets = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("<sheet") {
        let tag_start = cursor + offset;
        let after_name = &text[tag_start + "<sheet".len()..];
        if after_name.starts_with('s') {
            cursor = tag_start + "<sheet".len();
            continue;
        }
        let tag_end = text[tag_start..]
            .find('>')
            .map(|offset| tag_start + offset + 1)
            .ok_or_else(|| writer_error("unterminated workbook sheet tag"))?;
        let tag = &text[tag_start..tag_end];
        let name = tag_attribute(tag, "name")
            .ok_or_else(|| writer_error("workbook sheet is missing its name"))?;
        let sheet_id = tag_attribute(tag, "sheetId")
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| writer_error("workbook sheet is missing its sheetId"))?;
        let relationship_id = tag_attribute(tag, "r:id")
            .ok_or_else(|| writer_error("workbook sheet is missing its relationship ID"))?;
        let visible = !matches!(
            tag_attribute(tag, "state").as_deref(),
            Some("hidden" | "veryHidden")
        );
        sheets.push(Sheet {
            name: unescape_xml(&name)?,
            sheet_id,
            relationship_id,
            visible,
            tag_start,
            tag_end,
        });
        cursor = tag_end;
    }
    Ok(sheets)
}

fn find_sheet<'a>(sheets: &'a [Sheet], name: &str) -> Result<&'a Sheet> {
    sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| writer_error(format!("worksheet `{name}` was not found")))
}

fn worksheet_paths(package: &[u8], workbook: &[u8]) -> Result<Vec<String>> {
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels")?;
    let targets = relationship_targets(&relationships)?;
    sheets(workbook)?
        .into_iter()
        .map(|sheet| {
            targets
                .get(&sheet.relationship_id)
                .map(|target| normalize_target(target))
                .ok_or_else(|| {
                    writer_error(format!(
                        "workbook relationship `{}` is missing",
                        sheet.relationship_id
                    ))
                })
        })
        .collect()
}

fn worksheet_paths_with_names(package: &[u8], workbook: &[u8]) -> Result<Vec<(String, String)>> {
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels")?;
    let targets = relationship_targets(&relationships)?;
    sheets(workbook)?
        .into_iter()
        .map(|sheet| {
            let target = targets.get(&sheet.relationship_id).ok_or_else(|| {
                writer_error(format!(
                    "workbook relationship `{}` is missing",
                    sheet.relationship_id
                ))
            })?;
            Ok((sheet.name, normalize_target(target)))
        })
        .collect()
}

fn relationship_targets(xml: &[u8]) -> Result<BTreeMap<String, String>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut targets = BTreeMap::new();
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
                    targets.insert(id, target);
                }
            }
            Event::Eof => return Ok(targets),
            _ => {}
        }
        buffer.clear();
    }
}

fn relationship_ids(xml: &[u8]) -> Result<BTreeSet<u32>> {
    Ok(relationship_targets(xml)?
        .keys()
        .filter_map(|id| id.strip_prefix("rId")?.parse().ok())
        .collect())
}

fn worksheet_part_numbers(package: &[u8]) -> Result<BTreeSet<u32>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| writer_error(format!("invalid XLSX package: {error}")))?;
    let mut numbers = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| writer_error(format!("cannot read ZIP entry: {error}")))?;
        if let Some(number) = entry
            .name()
            .strip_prefix("xl/worksheets/sheet")
            .and_then(|name| name.strip_suffix(".xml"))
            .and_then(|number| number.parse().ok())
        {
            numbers.insert(number);
        }
    }
    Ok(numbers)
}

fn lowest_unused(used: &BTreeSet<u32>, prefix: &str) -> String {
    format!("{prefix}{}", lowest_unused_number(used))
}

fn lowest_unused_number(used: &BTreeSet<u32>) -> u32 {
    let mut number = 1;
    while used.contains(&number) {
        number += 1;
    }
    number
}

fn insert_sheet(xml: &[u8], tag: &str, sheets: &[Sheet], index: usize) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    let position = if index == 0 {
        text.find("<sheets")
            .and_then(|start| text[start..].find('>').map(|offset| start + offset + 1))
            .ok_or_else(|| writer_error("workbook XML is missing sheets"))?
    } else {
        sheets[index - 1].tag_end
    };
    Ok(format!("{}{}{}", &text[..position], tag, &text[position..]).into_bytes())
}

fn replace_tag_attribute(
    xml: &[u8],
    tag_start: usize,
    tag_end: usize,
    attribute: &str,
    replacement: &str,
) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    let tag = &text[tag_start..tag_end];
    let needle = format!("{attribute}=\"");
    let attribute_start = tag
        .find(&needle)
        .map(|offset| tag_start + offset + needle.len())
        .ok_or_else(|| writer_error(format!("workbook sheet is missing `{attribute}`")))?;
    let attribute_end = text[attribute_start..]
        .find('"')
        .map(|offset| attribute_start + offset)
        .ok_or_else(|| writer_error(format!("unterminated `{attribute}` attribute")))?;
    Ok(format!(
        "{}{}{}",
        &text[..attribute_start],
        escape_xml(replacement),
        &text[attribute_end..]
    )
    .into_bytes())
}

fn rewrite_worksheet_formulas(xml: &[u8], from: &str, to: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("<f") {
        let tag_start = cursor + offset;
        output.push_str(&text[cursor..tag_start]);
        let content_start = text[tag_start..]
            .find('>')
            .map(|offset| tag_start + offset + 1)
            .ok_or_else(|| writer_error("unterminated formula tag"))?;
        let content_end = text[content_start..]
            .find("</f>")
            .map(|offset| content_start + offset)
            .ok_or_else(|| writer_error("unterminated formula"))?;
        output.push_str(&text[tag_start..content_start]);
        output.push_str(&rewrite_formula(
            &text[content_start..content_end],
            from,
            to,
        ));
        output.push_str("</f>");
        cursor = content_end + "</f>".len();
    }
    output.push_str(&text[cursor..]);
    Ok(output.into_bytes())
}

fn rewrite_formula(formula: &str, from: &str, to: &str) -> String {
    let mut output = String::with_capacity(formula.len());
    let mut cursor = 0;
    for reference in reference_spans(formula) {
        if !reference
            .reference
            .sheet
            .as_deref()
            .is_some_and(|sheet| sheet.eq_ignore_ascii_case(from))
        {
            continue;
        }
        output.push_str(&formula[cursor..reference.span.start]);
        let suffix = &formula[reference.span.start..reference.span.end];
        if to == "#REF!" {
            output.push_str("#REF!");
        } else if let Some((_, address)) = suffix.split_once('!') {
            output.push_str(&format!(
                "{}!{address}",
                sheet_reference_prefix(to).trim_end_matches('!')
            ));
        } else {
            output.push_str(suffix);
        }
        cursor = reference.span.end;
    }
    output.push_str(&formula[cursor..]);
    output
}

fn formula_cells_referencing_sheet(xml: &[u8], sheet: &str) -> Result<Vec<String>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    let mut references = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("<c ") {
        let cell_start = cursor + offset;
        let cell_end = text[cell_start..]
            .find("</c>")
            .map(|offset| cell_start + offset + "</c>".len())
            .ok_or_else(|| writer_error("unterminated worksheet cell"))?;
        let cell = &text[cell_start..cell_end];
        if let Some(formula_start) = cell.find("<f") {
            let content_start = cell[formula_start..]
                .find('>')
                .map(|offset| cell_start + formula_start + offset + 1)
                .ok_or_else(|| writer_error("unterminated formula tag"))?;
            let content_end = text[content_start..cell_end]
                .find("</f>")
                .map(|offset| content_start + offset)
                .ok_or_else(|| writer_error("unterminated formula"))?;
            if formula_references_sheet(&text[content_start..content_end], sheet) {
                let tag_end = cell
                    .find('>')
                    .map(|offset| cell_start + offset)
                    .ok_or_else(|| writer_error("unterminated cell tag"))?;
                if let Some(address) = tag_attribute(&text[cell_start..=tag_end], "r") {
                    references.push(address);
                }
            }
        }
        cursor = cell_end;
    }
    Ok(references)
}

fn upsert_defined_name(xml: &[u8], name: &str, formula: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    let escaped_name = escape_xml(name);
    let escaped_formula = escape_xml(formula);
    let tag = format!(r#"<definedName name="{escaped_name}">{escaped_formula}</definedName>"#);

    if let Some((open_start, _open_end, close_end)) = find_defined_name(text, name)? {
        return Ok(format!("{}{}{}", &text[..open_start], tag, &text[close_end..]).into_bytes());
    }

    if let Some(container_end) = find_defined_names_close(text)? {
        return Ok(format!(
            "{}{}{}",
            &text[..container_end],
            tag,
            &text[container_end..]
        )
        .into_bytes());
    }

    let sheets_close = text
        .find("</sheets>")
        .map(|offset| offset + "</sheets>".len())
        .ok_or_else(|| writer_error("workbook XML is missing </sheets>"))?;
    Ok(format!(
        "{}<definedNames>{}</definedNames>{}",
        &text[..sheets_close],
        tag,
        &text[sheets_close..]
    )
    .into_bytes())
}

fn remove_defined_name(xml: &[u8], name: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    let Some((open_start, _open_end, close_end)) = find_defined_name(text, name)? else {
        return Err(writer_error(format!("named range `{name}` was not found")));
    };
    let mut patched = format!("{}{}", &text[..open_start], &text[close_end..]);
    // Drop empty <definedNames>...</definedNames> container when last name is removed.
    if let Some(start) = patched.find("<definedNames") {
        let open_end = patched[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error("unterminated definedNames"))?;
        if !patched[start..open_end].ends_with("/>") {
            let close = patched[open_end..]
                .find("</definedNames>")
                .map(|offset| open_end + offset)
                .ok_or_else(|| writer_error("unterminated definedNames"))?;
            let close_end = close + "</definedNames>".len();
            let inner = patched[open_end..close].trim();
            if inner.is_empty() {
                patched = format!("{}{}", &patched[..start], &patched[close_end..]);
            }
        }
    }
    Ok(patched.into_bytes())
}

fn find_defined_name(text: &str, name: &str) -> Result<Option<(usize, usize, usize)>> {
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("<definedName") {
        let start = cursor + offset;
        if text[start + "<definedName".len()..].starts_with('s') {
            cursor = start + "<definedName".len();
            continue;
        }
        let open_end = text[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error("unterminated defined name"))?;
        let open_tag = &text[start..open_end];
        if open_tag.ends_with("/>") {
            cursor = open_end;
            continue;
        }
        let close = text[open_end..]
            .find("</definedName>")
            .map(|offset| open_end + offset)
            .ok_or_else(|| writer_error("unterminated defined name"))?;
        let close_end = close + "</definedName>".len();
        if tag_attribute(open_tag, "name")
            .as_deref()
            .is_some_and(|existing| existing.eq_ignore_ascii_case(name))
        {
            return Ok(Some((start, open_end, close_end)));
        }
        cursor = close_end;
    }
    Ok(None)
}

fn find_defined_names_close(text: &str) -> Result<Option<usize>> {
    let Some(start) = text.find("<definedNames") else {
        return Ok(None);
    };
    let open_end = text[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| writer_error("unterminated definedNames"))?;
    if text[start..open_end].ends_with("/>") {
        return Ok(None);
    }
    text[open_end..]
        .find("</definedNames>")
        .map(|offset| Some(open_end + offset))
        .ok_or_else(|| writer_error("unterminated definedNames"))
}

fn defined_names_referencing_sheet(xml: &[u8], sheet: &str) -> Result<Vec<String>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    let mut names = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("<definedName") {
        let start = cursor + offset;
        if text[start + "<definedName".len()..].starts_with('s') {
            cursor = start + "<definedName".len();
            continue;
        }
        let open_end = text[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error("unterminated defined name"))?;
        let end = text[open_end..]
            .find("</definedName>")
            .map(|offset| open_end + offset)
            .ok_or_else(|| writer_error("unterminated defined name"))?;
        if formula_references_sheet(&text[open_end..end], sheet) {
            names.push(
                tag_attribute(&text[start..open_end], "name").unwrap_or_else(|| "<unnamed>".into()),
            );
        }
        cursor = end + "</definedName>".len();
    }
    Ok(names)
}

fn formula_references_sheet(formula: &str, sheet: &str) -> bool {
    reference_spans(formula).iter().any(|reference| {
        reference
            .reference
            .sheet
            .as_deref()
            .is_some_and(|referenced| referenced.eq_ignore_ascii_case(sheet))
    })
}

fn rewrite_defined_name_formulas(xml: &[u8], from: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("<definedName") {
        let start = cursor + offset;
        if text[start + "<definedName".len()..].starts_with('s') {
            output.push_str(&text[cursor..start + "<definedName".len()]);
            cursor = start + "<definedName".len();
            continue;
        }
        let open_end = text[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error("unterminated defined name"))?;
        let end = text[open_end..]
            .find("</definedName>")
            .map(|offset| open_end + offset)
            .ok_or_else(|| writer_error("unterminated defined name"))?;
        output.push_str(&text[cursor..open_end]);
        output.push_str(&rewrite_formula(&text[open_end..end], from, "#REF!"));
        output.push_str("</definedName>");
        cursor = end + "</definedName>".len();
    }
    output.push_str(&text[cursor..]);
    Ok(output.into_bytes())
}

fn add_app_sheet_title(xml: &[u8], name: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("app properties XML is not UTF-8: {error}")))?;
    patch_app_titles(text, |titles| {
        titles.push(name.to_owned());
    })
}

fn remove_app_sheet_title(xml: &[u8], name: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("app properties XML is not UTF-8: {error}")))?;
    patch_app_titles(text, |titles| {
        if let Some(index) = titles
            .iter()
            .position(|title| title.eq_ignore_ascii_case(name))
        {
            titles.remove(index);
        }
    })
}

fn rename_app_sheet_title(xml: &[u8], from: &str, to: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("app properties XML is not UTF-8: {error}")))?;
    patch_app_titles(text, |titles| {
        if let Some(title) = titles
            .iter_mut()
            .find(|title| title.eq_ignore_ascii_case(from))
        {
            *title = to.to_owned();
        }
    })
}

fn patch_app_titles(text: &str, update: impl FnOnce(&mut Vec<String>)) -> Result<Vec<u8>> {
    let titles_start = match text.find("<TitlesOfParts>") {
        Some(start) => start,
        None => return Ok(text.as_bytes().to_vec()),
    };
    let vector_start = text[titles_start..]
        .find("<vt:vector")
        .map(|offset| titles_start + offset)
        .ok_or_else(|| writer_error("app properties TitlesOfParts is missing its vector"))?;
    let vector_open_end = text[vector_start..]
        .find('>')
        .map(|offset| vector_start + offset + 1)
        .ok_or_else(|| writer_error("unterminated app properties vector"))?;
    let vector_end = text[vector_open_end..]
        .find("</vt:vector>")
        .map(|offset| vector_open_end + offset)
        .ok_or_else(|| writer_error("unterminated app properties vector"))?;
    let mut titles = text[vector_open_end..vector_end]
        .split("<vt:lpstr>")
        .skip(1)
        .filter_map(|title| {
            title
                .split_once("</vt:lpstr>")
                .map(|(title, _)| title.to_owned())
        })
        .collect::<Vec<_>>();
    update(&mut titles);
    let open = &text[vector_start..vector_open_end];
    let open = replace_attribute(open, "size", &titles.len().to_string())?;
    let titles = titles
        .iter()
        .map(|title| format!("<vt:lpstr>{}</vt:lpstr>", escape_xml(title)))
        .collect::<String>();
    Ok(format!(
        "{}{}{}{}",
        &text[..vector_start],
        open,
        titles,
        &text[vector_end..]
    )
    .into_bytes())
}

fn replace_attribute(tag: &str, attribute: &str, replacement: &str) -> Result<String> {
    let needle = format!("{attribute}=\"");
    let start = tag
        .find(&needle)
        .map(|offset| offset + needle.len())
        .ok_or_else(|| writer_error(format!("app properties vector is missing `{attribute}`")))?;
    let end = tag[start..]
        .find('"')
        .map(|offset| start + offset)
        .ok_or_else(|| writer_error(format!("unterminated `{attribute}` attribute")))?;
    Ok(format!("{}{}{}", &tag[..start], replacement, &tag[end..]))
}

fn remove_span(xml: &[u8], start: usize, end: usize) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("XML is not UTF-8: {error}")))?;
    Ok(format!("{}{}", &text[..start], &text[end..]).into_bytes())
}

fn remove_relationship(xml: &[u8], id: &str) -> Result<Vec<u8>> {
    remove_matching_tag(xml, "Relationship", |tag| {
        tag_attribute(tag, "Id").as_deref() == Some(id)
    })
}

fn remove_content_type_overrides(xml: &[u8], parts: &BTreeSet<String>) -> Result<Vec<u8>> {
    remove_matching_tag(xml, "Override", |tag| {
        tag_attribute(tag, "PartName")
            .is_some_and(|part| parts.contains(part.trim_start_matches('/')))
    })
}

fn remove_matching_tag(
    xml: &[u8],
    tag_name: &str,
    matches: impl Fn(&str) -> bool,
) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("XML is not UTF-8: {error}")))?;
    let needle = format!("<{tag_name}");
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find(&needle) {
        let start = cursor + offset;
        let after_name = &text[start + needle.len()..];
        if after_name.starts_with(|character: char| character.is_ascii_alphabetic()) {
            output.push_str(&text[cursor..start + needle.len()]);
            cursor = start + needle.len();
            continue;
        }
        let end = text[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| writer_error(format!("unterminated {tag_name} tag")))?;
        output.push_str(&text[cursor..start]);
        if !matches(&text[start..end]) {
            output.push_str(&text[start..end]);
        }
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    Ok(output.into_bytes())
}

fn relationship_part_name(part: &str) -> String {
    let (directory, name) = part.rsplit_once('/').unwrap_or(("", part));
    if directory.is_empty() {
        format!("_rels/{name}.rels")
    } else {
        format!("{directory}/_rels/{name}.rels")
    }
}

fn relationship_source(part: &str) -> Option<String> {
    let (directory, file) = part.rsplit_once("/_rels/")?;
    let name = file.strip_suffix(".rels")?;
    Some(format!("{directory}/{name}"))
}

fn orphaned_visual_parts(package: &[u8], deleted_relationships: &str) -> Result<BTreeSet<String>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| writer_error(format!("invalid XLSX package: {error}")))?;
    let mut graph = BTreeMap::<String, Vec<String>>::new();
    let mut candidates = BTreeSet::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| writer_error(format!("cannot read ZIP entry: {error}")))?;
        let relationship_part = entry.name().to_owned();
        if !relationship_part.ends_with(".rels") {
            continue;
        }
        let Some(source) = relationship_source(&relationship_part) else {
            continue;
        };
        let mut xml = Vec::new();
        entry
            .read_to_end(&mut xml)
            .map_err(|error| writer_error(format!("cannot read `{relationship_part}`: {error}")))?;
        let targets = relationship_targets(&xml)?
            .into_values()
            .map(|target| resolve_relationship_target(&source, &target))
            .collect::<Vec<_>>();
        if relationship_part == deleted_relationships {
            candidates.extend(
                targets
                    .iter()
                    .filter(|target| target.starts_with("xl/drawings/"))
                    .cloned(),
            );
        }
        graph.insert(source, targets);
    }

    let deleted_source = relationship_source(deleted_relationships).unwrap_or_default();
    let mut removals = BTreeSet::new();
    loop {
        let mut changed = false;
        for candidate in candidates.clone() {
            if removals.contains(&candidate)
                || graph.iter().any(|(source, targets)| {
                    !removals.contains(source)
                        && source != &deleted_source
                        && targets.iter().any(|target| target == &candidate)
                })
            {
                continue;
            }
            removals.insert(candidate.clone());
            removals.insert(relationship_part_name(&candidate));
            if let Some(targets) = graph.get(&candidate) {
                candidates.extend(
                    targets
                        .iter()
                        .filter(|target| {
                            target.starts_with("xl/drawings/") || target.starts_with("xl/charts/")
                        })
                        .cloned(),
                );
            }
            changed = true;
        }
        if !changed {
            return Ok(removals);
        }
    }
}

fn resolve_relationship_target(source: &str, target: &str) -> String {
    if target.starts_with('/') {
        return target.trim_start_matches('/').into();
    }
    let base = source
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    let mut components = base
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            component => components.push(component),
        }
    }
    components.join("/")
}

fn insert_before_close(xml: &[u8], element: &str, insertion: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("XML is not UTF-8: {error}")))?;
    let closing = format!("</{element}>");
    let position = text
        .rfind(&closing)
        .ok_or_else(|| writer_error(format!("XML is missing closing `{element}`")))?;
    Ok(format!("{}{}{}", &text[..position], insertion, &text[position..]).into_bytes())
}

fn tag_attribute(tag: &str, attribute: &str) -> Option<String> {
    let needle = format!("{attribute}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

fn normalize_target(target: &str) -> String {
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

fn sheet_reference_prefix(name: &str) -> String {
    if name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '.'))
        && !name.is_empty()
    {
        format!("{name}!")
    } else {
        format!("'{}'!", name.replace('\'', "''"))
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unescape_xml(value: &str) -> Result<String> {
    quick_xml::escape::unescape(value)
        .map(|value| value.into_owned())
        .map_err(|error| writer_error(format!("invalid XML value: {error}")))
}

fn writer_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx workbook writer>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::validate_rename_safety;

    #[test]
    fn rename_safety_rejects_quoted_sheet_references_in_charts() {
        let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file("xl/charts/chart1.xml", SimpleFileOptions::default())
            .expect("start chart entry");
        writer
            .write_all(
                br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:f>'Inputs'!$A$1:$A$2</c:f></c:chartSpace>"#,
            )
            .expect("write chart entry");
        let package = writer.finish().expect("finish package").into_inner();

        let error = validate_rename_safety(&package, "Inputs")
            .expect_err("quoted chart references must block sheet rename");

        assert!(error.to_string().contains("xl/charts/chart1.xml"));
    }
}
