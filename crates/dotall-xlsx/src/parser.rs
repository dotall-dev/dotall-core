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
    CellModel, CellValue, NamedRange, PageMargins, SCHEMA_VERSION, SheetDimensions, SheetModel,
    StyleEntry, UnmodeledMap, WorkbookModel, column_name,
};

/// Parsed `xl/styles.xml` cellXfs + number-format resolution.
struct StyleCatalog {
    /// `cellXfs` index → resolved Excel number-format code (`None` = General / unset).
    number_formats: Vec<Option<String>>,
}

#[derive(Clone, Debug)]
struct WorksheetPart {
    merges: Vec<String>,
    freeze_panes: Option<String>,
    tab_color: Option<String>,
    auto_filter: Option<String>,
    page_orientation: Option<String>,
    page_margins: Option<PageMargins>,
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
        .filter(|(name, _)| !name.starts_with("_xlnm."))
        .map(|(name, formula)| NamedRange {
            element_id: ids::named_range_id(name, formula, SCHEMA_VERSION),
            name: name.to_owned(),
            formula: formula.to_owned(),
        })
        .collect();

    let worksheet_parts = parse_worksheet_parts(&source_bytes, source)?;
    let print_areas = parse_print_areas(&source_bytes, source)?;
    let print_titles_map = parse_print_titles(&source_bytes, source)?;
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
                    freeze_panes: None,
                    tab_color: None,
                    auto_filter: None,
                    page_orientation: None,
                    page_margins: None,
                    cell_styles: BTreeMap::new(),
                });
            let print_area = print_areas.get(&name).cloned();
            let print_titles = print_titles_map.get(&name).cloned();
            parse_sheet(
                &mut workbook,
                source,
                name,
                index as u32,
                part,
                PrintExtras {
                    print_area,
                    print_titles,
                },
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
    print: PrintExtras,
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
        freeze_panes: part.freeze_panes,
        tab_color: part.tab_color,
        auto_filter: part.auto_filter,
        print_area: print.print_area,
        print_titles: print.print_titles,
        page_orientation: part.page_orientation,
        page_margins: part.page_margins,
        cells,
    })
}

struct PrintExtras {
    print_area: Option<String>,
    print_titles: Option<crate::model::PrintTitles>,
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
        let freeze_panes = parse_freeze_panes(&worksheet, source)?;
        let tab_color = parse_tab_color(&worksheet, source)?;
        let auto_filter = parse_auto_filter(&worksheet, source)?;
        let page_orientation = parse_page_orientation(&worksheet, source)?;
        let page_margins = parse_page_margins(&worksheet, source)?;
        let cell_styles = parse_cell_style_indices(&worksheet, source)?;
        parts.insert(
            name,
            WorksheetPart {
                merges,
                freeze_panes,
                tab_color,
                auto_filter,
                page_orientation,
                page_margins,
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

fn parse_tab_color(xml: &[u8], source: &Path) -> Result<Option<String>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"tabColor" =>
            {
                for attribute in element.attributes().flatten() {
                    if local_name(attribute.key.as_ref()) == b"rgb" {
                        let rgb = String::from_utf8_lossy(attribute.value.as_ref())
                            .trim()
                            .to_ascii_uppercase();
                        if !rgb.is_empty() {
                            return Ok(Some(rgb));
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(None)
}

fn parse_auto_filter(xml: &[u8], source: &Path) -> Result<Option<String>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"autoFilter" =>
            {
                for attribute in element.attributes().flatten() {
                    if local_name(attribute.key.as_ref()) == b"ref" {
                        let reference = String::from_utf8_lossy(attribute.value.as_ref())
                            .trim()
                            .to_ascii_uppercase();
                        if !reference.is_empty() {
                            return Ok(Some(reference));
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(None)
}

fn parse_page_orientation(xml: &[u8], source: &Path) -> Result<Option<String>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"pageSetup" =>
            {
                for attribute in element.attributes().flatten() {
                    if local_name(attribute.key.as_ref()) == b"orientation" {
                        let value = String::from_utf8_lossy(attribute.value.as_ref())
                            .trim()
                            .to_ascii_lowercase();
                        if value == "portrait" || value == "landscape" {
                            return Ok(Some(value));
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(None)
}

fn parse_page_margins(xml: &[u8], source: &Path) -> Result<Option<PageMargins>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"pageMargins" =>
            {
                let mut left = None;
                let mut right = None;
                let mut top = None;
                let mut bottom = None;
                let mut header = None;
                let mut footer = None;
                for attribute in element.attributes().flatten() {
                    let value = String::from_utf8_lossy(attribute.value.as_ref())
                        .trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite() && *v >= 0.0);
                    match local_name(attribute.key.as_ref()) {
                        b"left" => left = value,
                        b"right" => right = value,
                        b"top" => top = value,
                        b"bottom" => bottom = value,
                        b"header" => header = value,
                        b"footer" => footer = value,
                        _ => {}
                    }
                }
                if let (Some(left), Some(right), Some(top), Some(bottom)) =
                    (left, right, top, bottom)
                {
                    return Ok(Some(PageMargins {
                        left,
                        right,
                        top,
                        bottom,
                        header,
                        footer,
                    }));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(None)
}

fn parse_print_areas(package: &[u8], source: &Path) -> Result<BTreeMap<String, String>> {
    let workbook = entry_bytes(package, "xl/workbook.xml", source)?;
    let sheet_names: Vec<String> = parse_workbook_sheets(&workbook, source)?
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let mut areas = BTreeMap::new();
    let mut reader = XmlReader::from_reader(workbook.as_slice());
    let mut buffer = Vec::new();
    let mut current_local_sheet: Option<u32> = None;
    let mut in_print_area = false;
    let mut formula = String::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid workbook XML: {error}")))?
        {
            Event::Start(element) if local_name(element.name().as_ref()) == b"definedName" => {
                let mut name = None;
                let mut local_sheet_id = None;
                for attribute in element.attributes().flatten() {
                    match local_name(attribute.key.as_ref()) {
                        b"name" => {
                            name = Some(
                                String::from_utf8_lossy(attribute.value.as_ref()).into_owned(),
                            );
                        }
                        b"localSheetId" => {
                            local_sheet_id = parse_u32_attr(attribute.value.as_ref());
                        }
                        _ => {}
                    }
                }
                if name.as_deref() == Some("_xlnm.Print_Area") {
                    in_print_area = true;
                    current_local_sheet = local_sheet_id;
                    formula.clear();
                }
            }
            Event::Text(text) if in_print_area => {
                let decoded = String::from_utf8_lossy(text.as_ref());
                match quick_xml::escape::unescape(&decoded) {
                    Ok(unescaped) => formula.push_str(&unescaped),
                    Err(_) => formula.push_str(&decoded),
                }
            }
            Event::End(element)
                if in_print_area && local_name(element.name().as_ref()) == b"definedName" =>
            {
                if let Some(local_id) = current_local_sheet
                    && let Some(sheet_name) = sheet_names.get(local_id as usize)
                    && let Some(range) = normalize_print_area_formula(&formula)
                {
                    areas.insert(sheet_name.clone(), range);
                }
                in_print_area = false;
                current_local_sheet = None;
                formula.clear();
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(areas)
}

fn normalize_print_area_formula(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Accept `Sheet!$A$1:$B$2`, `'Sheet Name'!$A$1:$B$2`, or bare `A1:B2`.
    let range_part = trimmed
        .rsplit_once('!')
        .map(|(_, range)| range)
        .unwrap_or(trimmed)
        .trim()
        .replace('$', "");
    if range_part.is_empty() {
        return None;
    }
    Some(range_part.to_ascii_uppercase())
}

fn parse_print_titles(
    package: &[u8],
    source: &Path,
) -> Result<BTreeMap<String, crate::model::PrintTitles>> {
    let workbook = entry_bytes(package, "xl/workbook.xml", source)?;
    let sheet_names: Vec<String> = parse_workbook_sheets(&workbook, source)?
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let mut titles = BTreeMap::new();
    let mut reader = XmlReader::from_reader(workbook.as_slice());
    let mut buffer = Vec::new();
    let mut current_local_sheet: Option<u32> = None;
    let mut in_print_titles = false;
    let mut formula = String::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid workbook XML: {error}")))?
        {
            Event::Start(element) if local_name(element.name().as_ref()) == b"definedName" => {
                let mut name = None;
                let mut local_sheet_id = None;
                for attribute in element.attributes().flatten() {
                    match local_name(attribute.key.as_ref()) {
                        b"name" => {
                            name = Some(
                                String::from_utf8_lossy(attribute.value.as_ref()).into_owned(),
                            );
                        }
                        b"localSheetId" => {
                            local_sheet_id = parse_u32_attr(attribute.value.as_ref());
                        }
                        _ => {}
                    }
                }
                if name.as_deref() == Some("_xlnm.Print_Titles") {
                    in_print_titles = true;
                    current_local_sheet = local_sheet_id;
                    formula.clear();
                }
            }
            Event::Text(text) if in_print_titles => {
                let decoded = String::from_utf8_lossy(text.as_ref());
                match quick_xml::escape::unescape(&decoded) {
                    Ok(unescaped) => formula.push_str(&unescaped),
                    Err(_) => formula.push_str(&decoded),
                }
            }
            Event::End(element)
                if in_print_titles && local_name(element.name().as_ref()) == b"definedName" =>
            {
                if let Some(local_id) = current_local_sheet
                    && let Some(sheet_name) = sheet_names.get(local_id as usize)
                    && let Some(parsed) = normalize_print_titles_formula(&formula)
                {
                    titles.insert(sheet_name.clone(), parsed);
                }
                in_print_titles = false;
                current_local_sheet = None;
                formula.clear();
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(titles)
}

fn normalize_print_titles_formula(raw: &str) -> Option<crate::model::PrintTitles> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut rows = None;
    let mut cols = None;
    for part in trimmed.split(',') {
        let range_part = part
            .rsplit_once('!')
            .map(|(_, range)| range)
            .unwrap_or(part)
            .trim()
            .replace('$', "");
        if range_part.is_empty() {
            continue;
        }
        let upper = range_part.to_ascii_uppercase();
        if upper
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            rows = Some(upper);
        } else {
            cols = Some(upper);
        }
    }
    if rows.is_none() && cols.is_none() {
        return None;
    }
    Some(crate::model::PrintTitles { rows, cols })
}

fn parse_freeze_panes(xml: &[u8], source: &Path) -> Result<Option<String>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"pane" =>
            {
                let mut state = None;
                let mut top_left = None;
                let mut x_split = 0.0_f64;
                let mut y_split = 0.0_f64;
                for attribute in element.attributes().flatten() {
                    match local_name(attribute.key.as_ref()) {
                        b"state" => {
                            state =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        b"topLeftCell" => {
                            top_left =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        b"xSplit" => {
                            x_split = std::str::from_utf8(attribute.value.as_ref())
                                .ok()
                                .and_then(|value| value.parse().ok())
                                .unwrap_or(0.0);
                        }
                        b"ySplit" => {
                            y_split = std::str::from_utf8(attribute.value.as_ref())
                                .ok()
                                .and_then(|value| value.parse().ok())
                                .unwrap_or(0.0);
                        }
                        _ => {}
                    }
                }
                let frozen = state.as_deref().is_some_and(|value| {
                    value.eq_ignore_ascii_case("frozen")
                        || value.eq_ignore_ascii_case("frozenSplit")
                });
                if frozen {
                    if let Some(cell) = top_left.filter(|value| !value.is_empty()) {
                        return Ok(Some(cell.to_ascii_uppercase()));
                    }
                    let cols = x_split.floor().max(0.0) as u32;
                    let rows = y_split.floor().max(0.0) as u32;
                    if cols == 0 && rows == 0 {
                        return Ok(None);
                    }
                    return Ok(Some(format!("{}{}", column_name(cols as usize), rows + 1)));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(None)
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
