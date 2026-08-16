use std::collections::{BTreeMap, BTreeSet};
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
    CellModel, CellValue, NamedRange, SCHEMA_VERSION, SheetDimensions, SheetModel, StyleEntry,
    UnmodeledMap, WorkbookModel, column_name,
};

/// Parsed `xl/styles.xml` cellXfs + number-format resolution.
struct StyleCatalog {
    /// `cellXfs` index → resolved Excel number-format code (`None` = General / unset).
    number_formats: Vec<Option<String>>,
}

#[derive(Clone, Debug)]
struct WorksheetPart {
    merges: Vec<String>,
    cell_styles: BTreeMap<String, u32>,
}

struct SheetStyleContext<'a> {
    catalog: &'a StyleCatalog,
    used_indices: &'a mut BTreeSet<u32>,
}

/// Parses an XLSX workbook into the canonical `xlsx.workbook` v1 model.
///
/// Calamine does not expose worksheet merges or cell style indices through its
/// public API, so merges and style refs are read from OOXML (`styles.xml` + sheet
/// cell `s` attributes). Style entries stay id-only; full font/fill writers are
/// out of scope.
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

    let worksheet_parts = parse_worksheet_parts(&source_bytes, source)?;
    let style_catalog = parse_style_catalog(&source_bytes, source)?;
    let mut used_style_indices = BTreeSet::new();

    let sheets = workbook
        .sheet_names()
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let part = worksheet_parts
                .get(&name)
                .cloned()
                .unwrap_or_else(|| WorksheetPart {
                    merges: Vec::new(),
                    cell_styles: BTreeMap::new(),
                });
            parse_sheet(
                &mut workbook,
                source,
                name,
                index as u32,
                part,
                SheetStyleContext {
                    catalog: &style_catalog,
                    used_indices: &mut used_style_indices,
                },
            )
        })
        .collect::<Result<Vec<_>>>()?;

    let style_table = used_style_indices
        .into_iter()
        .map(|index| StyleEntry {
            style_id: ids::style_id(index, SCHEMA_VERSION),
        })
        .collect();

    Ok(WorkbookModel {
        workbook_id: ids::workbook_id(&source_hash, SCHEMA_VERSION),
        sheets,
        named_ranges,
        style_table,
        unmodeled: UnmodeledMap::default(),
    })
}

fn parse_sheet<RS, R>(
    workbook: &mut R,
    source: &Path,
    name: String,
    index: u32,
    part: WorksheetPart,
    styles: SheetStyleContext<'_>,
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
            let (style_id, number_format) = resolve_cell_style(
                &address,
                &part.cell_styles,
                styles.catalog,
                styles.used_indices,
            );

            CellModel {
                element_id: ids::cell_id(&name, &address, SCHEMA_VERSION),
                address,
                row,
                col,
                value,
                formula,
                style_id,
                number_format,
            }
        })
        .collect();

    Ok(SheetModel {
        element_id: ids::sheet_id(&name, index, SCHEMA_VERSION),
        name,
        index,
        dimensions,
        merges: part.merges,
        cells,
    })
}

fn resolve_cell_style(
    address: &str,
    cell_styles: &BTreeMap<String, u32>,
    style_catalog: &StyleCatalog,
    used_style_indices: &mut BTreeSet<u32>,
) -> (Option<String>, Option<String>) {
    let Some(&style_index) = cell_styles.get(address) else {
        return (None, None);
    };
    // Index 0 is the workbook default (typically General); omit unless agents need it.
    if style_index == 0 {
        return (None, None);
    }

    used_style_indices.insert(style_index);
    let style_id = ids::style_id(style_index, SCHEMA_VERSION);
    let number_format = style_catalog
        .number_formats
        .get(style_index as usize)
        .cloned()
        .flatten()
        .filter(|format| !format.eq_ignore_ascii_case("General"));
    (Some(style_id), number_format)
}

fn parse_worksheet_parts(package: &[u8], source: &Path) -> Result<BTreeMap<String, WorksheetPart>> {
    let workbook = entry_bytes(package, "xl/workbook.xml", source)?;
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels", source)?;
    let relationship_targets = parse_relationships(&relationships, source)?;
    let sheets = parse_workbook_sheets(&workbook, source)?;

    let mut parts = BTreeMap::new();
    for (name, relationship_id) in sheets {
        let target = relationship_targets.get(&relationship_id).ok_or_else(|| {
            format_error(
                source,
                format!("workbook relationship `{relationship_id}` not found for sheet `{name}`"),
            )
        })?;
        let path = normalize_relationship_target(target);
        let worksheet = entry_bytes(package, &path, source)?;
        let merges = parse_merge_refs(&worksheet, source)?;
        let cell_styles = parse_cell_style_indices(&worksheet, source)?;
        parts.insert(
            name,
            WorksheetPart {
                merges,
                cell_styles,
            },
        );
    }
    Ok(parts)
}

fn parse_style_catalog(package: &[u8], source: &Path) -> Result<StyleCatalog> {
    let Ok(styles) = entry_bytes(package, "xl/styles.xml", source) else {
        return Ok(StyleCatalog {
            number_formats: Vec::new(),
        });
    };

    let mut custom_formats = BTreeMap::new();
    let mut cell_xf_num_fmt_ids = Vec::new();
    let mut in_cell_xfs = false;

    let mut reader = XmlReader::from_reader(styles.as_slice());
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid styles XML: {error}")))?
        {
            Event::Start(element) if local_name(element.name().as_ref()) == b"cellXfs" => {
                in_cell_xfs = true;
            }
            Event::End(element) if local_name(element.name().as_ref()) == b"cellXfs" => {
                in_cell_xfs = false;
            }
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"numFmt" =>
            {
                let mut num_fmt_id = None;
                let mut format_code = None;
                for attribute in element.attributes().flatten() {
                    match local_name(attribute.key.as_ref()) {
                        b"numFmtId" => {
                            num_fmt_id = parse_u32_attr(attribute.value.as_ref());
                        }
                        b"formatCode" => {
                            format_code = Some(
                                quick_xml::escape::unescape(&String::from_utf8_lossy(
                                    attribute.value.as_ref(),
                                ))
                                .map_err(|error| {
                                    format_error(
                                        source,
                                        format!("invalid number format code: {error}"),
                                    )
                                })?
                                .into_owned(),
                            );
                        }
                        _ => {}
                    }
                }
                if let (Some(num_fmt_id), Some(format_code)) = (num_fmt_id, format_code) {
                    custom_formats.insert(num_fmt_id, format_code);
                }
            }
            Event::Empty(element) | Event::Start(element)
                if in_cell_xfs && local_name(element.name().as_ref()) == b"xf" =>
            {
                let mut num_fmt_id = 0_u32;
                for attribute in element.attributes().flatten() {
                    if local_name(attribute.key.as_ref()) == b"numFmtId"
                        && let Some(value) = parse_u32_attr(attribute.value.as_ref())
                    {
                        num_fmt_id = value;
                    }
                }
                cell_xf_num_fmt_ids.push(num_fmt_id);
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    let number_formats = cell_xf_num_fmt_ids
        .into_iter()
        .map(|num_fmt_id| resolve_number_format(num_fmt_id, &custom_formats))
        .collect();
    Ok(StyleCatalog { number_formats })
}

fn resolve_number_format(
    num_fmt_id: u32,
    custom_formats: &BTreeMap<u32, String>,
) -> Option<String> {
    if let Some(format) = custom_formats.get(&num_fmt_id) {
        return Some(format.clone());
    }
    builtin_number_format(num_fmt_id).map(str::to_owned)
}

fn builtin_number_format(num_fmt_id: u32) -> Option<&'static str> {
    Some(match num_fmt_id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "$#,##0_);($#,##0)",
        6 => "$#,##0_);[Red]($#,##0)",
        7 => "$#,##0.00_);($#,##0.00)",
        8 => "$#,##0.00_);[Red]($#,##0.00)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "mm-dd-yy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yy h:mm",
        37 => "#,##0;(#,##0)",
        38 => "#,##0;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

fn parse_cell_style_indices(xml: &[u8], source: &Path) -> Result<BTreeMap<String, u32>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut styles = BTreeMap::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"c" =>
            {
                let mut address = None;
                let mut style_index = None;
                for attribute in element.attributes().flatten() {
                    match local_name(attribute.key.as_ref()) {
                        b"r" => {
                            address =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        b"s" => style_index = parse_u32_attr(attribute.value.as_ref()),
                        _ => {}
                    }
                }
                if let (Some(address), Some(style_index)) = (address, style_index) {
                    styles.insert(address, style_index);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(styles)
}

fn parse_u32_attr(value: &[u8]) -> Option<u32> {
    std::str::from_utf8(value)
        .ok()
        .and_then(|value| value.parse().ok())
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
