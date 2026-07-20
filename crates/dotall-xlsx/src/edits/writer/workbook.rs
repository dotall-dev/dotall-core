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
    })
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
    })
}

pub(crate) fn validate_rename_safety(package: &[u8], from: &str) -> Result<()> {
    let reference = sheet_reference_prefix(from);
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
        if xml.contains(&reference) {
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
        sheets.push(Sheet {
            name: unescape_xml(&name)?,
            sheet_id,
            relationship_id,
            tag_start,
            tag_end,
        });
        cursor = tag_end;
    }
    Ok(sheets)
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
        if let Some((_, address)) = suffix.split_once('!') {
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

fn add_app_sheet_title(xml: &[u8], name: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("app properties XML is not UTF-8: {error}")))?;
    patch_app_titles(text, |titles| {
        titles.push(name.to_owned());
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
