use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

use calamine::{Data, Reader, open_workbook_auto};
use dotall_core::{DotallError, Result};
use quick_xml::Reader as XmlReader;
use quick_xml::events::Event;
use zip::ZipArchive;

use crate::FORMAT_ID;
use crate::ids;
use crate::model::{
    CellModel, CellValue, NamedRange, SCHEMA_VERSION, SheetDimensions, SheetModel, UnmodeledMap,
    WorkbookModel, column_name,
};

/// Parses an XLSX workbook into the canonical `xlsx.workbook` v1 model.
///
/// Calamine does not expose worksheet merges through its public API, so merges are
/// read from worksheet OOXML. Number-format/style tables remain empty until a fuller
/// OOXML style reader lands.
pub fn parse_workbook(source: &Path) -> Result<WorkbookModel> {
    let source_bytes = fs::read(source).map_err(|error| format_error(source, error))?;
    let source_hash = blake3::hash(&source_bytes).to_hex().to_string();
    let mut workbook = open_workbook_auto(source).map_err(|error| format_error(source, error))?;

    let named_ranges = workbook
        .defined_names()
        .iter()
        .map(|(name, formula)| NamedRange {
            element_id: ids::named_range_id(name, formula, SCHEMA_VERSION),
            name: name.to_owned(),
            formula: formula.to_owned(),
        })
        .collect();

    let merges_by_sheet = parse_merges_by_sheet(&source_bytes, source)?;

    let sheets = workbook
        .sheet_names()
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let merges = merges_by_sheet.get(&name).cloned().unwrap_or_default();
            parse_sheet(&mut workbook, source, name, index as u32, merges)
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(WorkbookModel {
        workbook_id: ids::workbook_id(&source_hash, SCHEMA_VERSION),
        sheets,
        named_ranges,
        style_table: Vec::new(),
        unmodeled: UnmodeledMap::default(),
    })
}

fn parse_sheet<RS, R>(
    workbook: &mut R,
    source: &Path,
    name: String,
    index: u32,
    merges: Vec<String>,
) -> Result<SheetModel>
where
    RS: Read + Seek,
    R: Reader<RS>,
    R::Error: std::fmt::Display,
{
    let values = workbook
        .worksheet_range(&name)
        .map_err(|error| format_error(source, error))?;
    let formulas = workbook
        .worksheet_formula(&name)
        .map_err(|error| format_error(source, error))?;

    let mut cells = BTreeMap::new();
    let value_start = values.start().unwrap_or((0, 0));
    for (relative_row, relative_col, value) in values.used_cells() {
        let row = value_start.0 + relative_row as u32;
        let col = value_start.1 + relative_col as u32;
        cells.insert((row, col), (cell_value(value), None));
    }

    let formula_start = formulas.start().unwrap_or((0, 0));
    for (relative_row, relative_col, formula) in formulas.used_cells() {
        let row = formula_start.0 + relative_row as u32;
        let col = formula_start.1 + relative_col as u32;
        let formula = canonical_formula(formula);
        cells
            .entry((row, col))
            .and_modify(|(_, existing_formula)| *existing_formula = Some(formula.clone()))
            .or_insert((CellValue::Empty, Some(formula)));
    }

    let dimensions = cells.keys().fold(
        SheetDimensions { rows: 0, cols: 0 },
        |dimensions, (row, col)| SheetDimensions {
            rows: dimensions.rows.max(row + 1),
            cols: dimensions.cols.max(col + 1),
        },
    );

    let cells = cells
        .into_iter()
        .map(|((zero_based_row, zero_based_col), (value, formula))| {
            let row = zero_based_row + 1;
            let col = zero_based_col + 1;
            let address = format!("{}{}", column_name(zero_based_col as usize), row);

            CellModel {
                element_id: ids::cell_id(&name, &address, SCHEMA_VERSION),
                address,
                row,
                col,
                value,
                formula,
                style_id: None,
                number_format: None,
            }
        })
        .collect();

    Ok(SheetModel {
        element_id: ids::sheet_id(&name, index, SCHEMA_VERSION),
        name,
        index,
        dimensions,
        merges,
        cells,
    })
}

fn parse_merges_by_sheet(package: &[u8], source: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    let workbook = entry_bytes(package, "xl/workbook.xml", source)?;
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels", source)?;
    let relationship_targets = parse_relationships(&relationships, source)?;
    let sheets = parse_workbook_sheets(&workbook, source)?;

    let mut merges_by_sheet = BTreeMap::new();
    for (name, relationship_id) in sheets {
        let target = relationship_targets.get(&relationship_id).ok_or_else(|| {
            format_error(
                source,
                format!("workbook relationship `{relationship_id}` not found for sheet `{name}`"),
            )
        })?;
        let path = normalize_relationship_target(target);
        let worksheet = entry_bytes(package, &path, source)?;
        merges_by_sheet.insert(name, parse_merge_refs(&worksheet, source)?);
    }
    Ok(merges_by_sheet)
}

fn parse_merge_refs(xml: &[u8], source: &Path) -> Result<Vec<String>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut merges = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"mergeCell" =>
            {
                for attribute in element.attributes().flatten() {
                    if local_name(attribute.key.as_ref()) == b"ref" {
                        merges.push(String::from_utf8_lossy(attribute.value.as_ref()).into_owned());
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(merges)
}

fn parse_workbook_sheets(xml: &[u8], source: &Path) -> Result<Vec<(String, String)>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut sheets = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid workbook XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"sheet" =>
            {
                let mut name = None;
                let mut relationship_id = None;
                for attribute in element.attributes().flatten() {
                    match local_name(attribute.key.as_ref()) {
                        b"name" => {
                            name = Some(
                                quick_xml::escape::unescape(&String::from_utf8_lossy(
                                    attribute.value.as_ref(),
                                ))
                                .map_err(|error| {
                                    format_error(source, format!("invalid worksheet name: {error}"))
                                })?
                                .into_owned(),
                            )
                        }
                        b"id" => {
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

fn parse_relationships(xml: &[u8], source: &Path) -> Result<BTreeMap<String, String>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut relationships = BTreeMap::new();
    loop {
        match reader.read_event_into(&mut buffer).map_err(|error| {
            format_error(
                source,
                format!("invalid workbook relationships XML: {error}"),
            )
        })? {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"Relationship" =>
            {
                let mut id = None;
                let mut target = None;
                for attribute in element.attributes().flatten() {
                    match local_name(attribute.key.as_ref()) {
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

fn entry_bytes(package: &[u8], name: &str, source: &Path) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(source, format!("invalid XLSX package: {error}")))?;
    let mut entry = archive.by_name(name).map_err(|error| {
        format_error(source, format!("XLSX package is missing `{name}`: {error}"))
    })?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| format_error(source, format!("cannot read `{name}`: {error}")))?;
    Ok(bytes)
}

fn normalize_relationship_target(target: &str) -> String {
    if target.starts_with('/') {
        target.trim_start_matches('/').to_owned()
    } else {
        format!("xl/{target}")
    }
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn canonical_formula(formula: &str) -> String {
    if formula.starts_with('=') {
        formula.to_owned()
    } else {
        format!("={formula}")
    }
}

fn cell_value(value: &Data) -> CellValue {
    match value {
        Data::Empty => CellValue::Empty,
        Data::String(value) => CellValue::String(value.clone()),
        Data::Float(value) => CellValue::Float(*value),
        Data::Int(value) => CellValue::Integer(*value),
        Data::Bool(value) => CellValue::Boolean(*value),
        Data::Error(value) => CellValue::Error(value.to_string()),
        Data::DateTime(value) => {
            let (year, month, day, hour, minute, second, millisecond) = value.to_ymd_hms_milli();
            let iso = if millisecond == 0 {
                format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
            } else {
                format!(
                    "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03}"
                )
            };
            CellValue::Datetime(iso)
        }
        Data::DateTimeIso(value) | Data::DurationIso(value) => CellValue::Datetime(value.clone()),
    }
}

fn format_error(source: &Path, error: impl std::fmt::Display) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.to_owned(),
        path: source.to_path_buf(),
        message: error.to_string(),
    }
}
