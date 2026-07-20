use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use dotall_core::{DotallError, PatchedOutput, Result, ValidatedEdit};
use quick_xml::Reader;
use quick_xml::events::Event;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::FORMAT_ID;
use crate::edits::{EditableValue, XlsxEditOp, parse_validated_operations};

use super::shared_strings;
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
    if operations
        .iter()
        .any(|operation| matches!(operation, XlsxEditOp::InsertRow { .. }))
    {
        return Err(DotallError::UnsupportedCapability {
            format_id: FORMAT_ID.into(),
            capability: "insert_row".into(),
            available: vec!["set_cell_value".into(), "set_cell_formula".into()],
        });
    }
    let worksheet_paths = worksheet_paths(&original)?;
    let mut grouped = BTreeMap::<String, Vec<XlsxEditOp>>::new();
    for operation in operations {
        let sheet = match &operation {
            XlsxEditOp::SetCellValue { sheet, .. } | XlsxEditOp::SetCellFormula { sheet, .. } => sheet,
            XlsxEditOp::InsertRow { .. } => unreachable!("structural operations return above"),
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

    let bytes = rebuild_package(&original, &replacements)?;
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

fn rebuild_package(original: &[u8], replacements: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(original))
        .map_err(|error| writer_error(format!("invalid XLSX package: {error}")))?;
    let output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(output);

    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| writer_error(format!("cannot read ZIP entry: {error}")))?;
        let name = entry.name().to_owned();
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

    writer
        .finish()
        .map_err(|error| writer_error(format!("cannot finish XLSX package: {error}")))
        .map(|cursor| cursor.into_inner())
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
