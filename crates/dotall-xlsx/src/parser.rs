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
    CellModel, CellValue, CenterOnPage, ChartModel, CommentModel, FitToPage, HeaderFooter,
    NamedRange, PageMargins, PictureModel, SCHEMA_VERSION, SheetDimensions, SheetModel, StyleEntry,
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
    freeze_panes: Option<String>,
    zoom: Option<u32>,
    show_gridlines: bool,
    right_to_left: bool,
    tab_color: Option<String>,
    auto_filter: Option<String>,
    page_orientation: Option<String>,
    paper_size: Option<u32>,
    print_scale: Option<u32>,
    fit_to_page: Option<FitToPage>,
    center_on_page: Option<CenterOnPage>,
    page_margins: Option<PageMargins>,
    header_footer: Option<HeaderFooter>,
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
/// cell `s` attributes). Style catalog entries stay id-only on read; font/fill
/// writers append fonts/fills/cellXfs surgically via `set_cell_font` /
/// `set_cell_fill`.
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
                    zoom: None,
                    show_gridlines: true,
                    right_to_left: false,
                    tab_color: None,
                    auto_filter: None,
                    page_orientation: None,
                    paper_size: None,
                    print_scale: None,
                    fit_to_page: None,
                    center_on_page: None,
                    page_margins: None,
                    header_footer: None,
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

    let comments = parse_comments(&source_bytes, source, &sheets)?;
    let charts = parse_charts(&source_bytes, source, &sheets)?;
    let pictures = parse_pictures(&source_bytes, source, &sheets)?;

    Ok(WorkbookModel {
        workbook_id: ids::workbook_id(&source_hash, SCHEMA_VERSION),
        sheets,
        named_ranges,
        style_table,
        comments,
        charts,
        pictures,
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
        zoom: part.zoom,
        show_gridlines: part.show_gridlines,
        right_to_left: part.right_to_left,
        tab_color: part.tab_color,
        auto_filter: part.auto_filter,
        print_area: print.print_area,
        print_titles: print.print_titles,
        page_orientation: part.page_orientation,
        paper_size: part.paper_size,
        print_scale: part.print_scale,
        fit_to_page: part.fit_to_page,
        center_on_page: part.center_on_page,
        page_margins: part.page_margins,
        header_footer: part.header_footer,
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
        let zoom = parse_sheet_zoom(&worksheet, source)?;
        let show_gridlines = parse_show_gridlines(&worksheet, source)?;
        let right_to_left = parse_right_to_left(&worksheet, source)?;
        let tab_color = parse_tab_color(&worksheet, source)?;
        let auto_filter = parse_auto_filter(&worksheet, source)?;
        let page_orientation = parse_page_orientation(&worksheet, source)?;
        let paper_size = parse_paper_size(&worksheet, source)?;
        let print_scale = parse_print_scale(&worksheet, source)?;
        let fit_to_page = parse_fit_to_page(&worksheet, source)?;
        let center_on_page = parse_center_on_page(&worksheet, source)?;
        let page_margins = parse_page_margins(&worksheet, source)?;
        let header_footer = parse_header_footer(&worksheet, source)?;
        let cell_styles = parse_cell_style_indices(&worksheet, source)?;
        parts.insert(
            name,
            WorksheetPart {
                merges,
                freeze_panes,
                zoom,
                show_gridlines,
                right_to_left,
                tab_color,
                auto_filter,
                page_orientation,
                paper_size,
                print_scale,
                fit_to_page,
                center_on_page,
                page_margins,
                header_footer,
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

fn parse_paper_size(xml: &[u8], source: &Path) -> Result<Option<u32>> {
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
                    if local_name(attribute.key.as_ref()) == b"paperSize"
                        && let Ok(value) = String::from_utf8_lossy(attribute.value.as_ref())
                            .trim()
                            .parse::<u32>()
                        && value > 0
                    {
                        return Ok(Some(value));
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

fn parse_print_scale(xml: &[u8], source: &Path) -> Result<Option<u32>> {
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
                    if local_name(attribute.key.as_ref()) == b"scale" {
                        let value = String::from_utf8_lossy(attribute.value.as_ref())
                            .trim()
                            .parse::<u32>()
                            .ok()
                            .filter(|scale| (10..=400).contains(scale));
                        if let Some(scale) = value {
                            return Ok(Some(scale));
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

fn parse_fit_to_page(xml: &[u8], source: &Path) -> Result<Option<FitToPage>> {
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
                let mut width = None;
                let mut height = None;
                for attribute in element.attributes().flatten() {
                    let key = local_name(attribute.key.as_ref());
                    let value = String::from_utf8_lossy(attribute.value.as_ref())
                        .trim()
                        .parse::<u32>()
                        .ok();
                    match key {
                        b"fitToWidth" => width = value,
                        b"fitToHeight" => height = value,
                        _ => {}
                    }
                }
                if width.is_some() || height.is_some() {
                    return Ok(Some(FitToPage { width, height }));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(None)
}

fn parse_center_on_page(xml: &[u8], source: &Path) -> Result<Option<CenterOnPage>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"printOptions" =>
            {
                let mut horizontal = false;
                let mut vertical = false;
                for attribute in element.attributes().flatten() {
                    let key = local_name(attribute.key.as_ref());
                    let raw = String::from_utf8_lossy(attribute.value.as_ref());
                    let enabled = matches!(raw.trim(), "1" | "true" | "TRUE");
                    match key {
                        b"horizontalCentered" => horizontal = enabled,
                        b"verticalCentered" => vertical = enabled,
                        _ => {}
                    }
                }
                if horizontal || vertical {
                    return Ok(Some(CenterOnPage {
                        horizontal,
                        vertical,
                    }));
                }
                return Ok(None);
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

fn parse_header_footer(xml: &[u8], source: &Path) -> Result<Option<HeaderFooter>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut in_header_footer = false;
    let mut in_odd_header = false;
    let mut in_odd_footer = false;
    let mut header = String::new();
    let mut footer = String::new();
    let mut saw_header = false;
    let mut saw_footer = false;
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Start(element) => match local_name(element.name().as_ref()) {
                b"headerFooter" => in_header_footer = true,
                b"oddHeader" if in_header_footer => {
                    in_odd_header = true;
                    saw_header = true;
                }
                b"oddFooter" if in_header_footer => {
                    in_odd_footer = true;
                    saw_footer = true;
                }
                _ => {}
            },
            Event::Empty(element) if in_header_footer => {
                match local_name(element.name().as_ref()) {
                    b"oddHeader" => saw_header = true,
                    b"oddFooter" => saw_footer = true,
                    _ => {}
                }
            }
            Event::Text(text) if in_odd_header => {
                append_unescaped_text(&mut header, text.as_ref());
            }
            Event::Text(text) if in_odd_footer => {
                append_unescaped_text(&mut footer, text.as_ref());
            }
            Event::GeneralRef(entity) if in_odd_header => {
                append_general_ref(&mut header, entity.as_ref());
            }
            Event::GeneralRef(entity) if in_odd_footer => {
                append_general_ref(&mut footer, entity.as_ref());
            }
            Event::End(element) => match local_name(element.name().as_ref()) {
                b"oddHeader" => in_odd_header = false,
                b"oddFooter" => in_odd_footer = false,
                b"headerFooter" => {
                    if saw_header || saw_footer {
                        return Ok(Some(HeaderFooter {
                            header: saw_header.then_some(header),
                            footer: saw_footer.then_some(footer),
                        }));
                    }
                    return Ok(None);
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(None)
}

fn append_unescaped_text(target: &mut String, raw: &[u8]) {
    let decoded = String::from_utf8_lossy(raw);
    match quick_xml::escape::unescape(&decoded) {
        Ok(unescaped) => target.push_str(&unescaped),
        Err(_) => target.push_str(&decoded),
    }
}

fn append_general_ref(target: &mut String, entity: &[u8]) {
    let decoded = String::from_utf8_lossy(entity);
    if let Some(value) = quick_xml::escape::resolve_xml_entity(&decoded) {
        target.push_str(value);
        return;
    }
    if let Some(digits) = decoded.strip_prefix('#') {
        let parsed = if let Some(hex) = digits
            .strip_prefix('x')
            .or_else(|| digits.strip_prefix('X'))
        {
            u32::from_str_radix(hex, 16).ok()
        } else {
            digits.parse().ok()
        };
        if let Some(character) = parsed.and_then(char::from_u32) {
            target.push(character);
            return;
        }
    }
    target.push('&');
    target.push_str(&decoded);
    target.push(';');
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

fn parse_sheet_zoom(xml: &[u8], source: &Path) -> Result<Option<u32>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"sheetView" =>
            {
                for attribute in element.attributes().flatten() {
                    if local_name(attribute.key.as_ref()) == b"zoomScale" {
                        let value = String::from_utf8_lossy(attribute.value.as_ref())
                            .trim()
                            .parse::<u32>()
                            .ok()
                            .filter(|zoom| (10..=400).contains(zoom));
                        if let Some(zoom) = value {
                            return Ok(Some(zoom));
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

fn parse_show_gridlines(xml: &[u8], source: &Path) -> Result<bool> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"sheetView" =>
            {
                for attribute in element.attributes().flatten() {
                    if local_name(attribute.key.as_ref()) == b"showGridLines" {
                        let value = String::from_utf8_lossy(attribute.value.as_ref());
                        let shown = !matches!(value.trim(), "0" | "false" | "off");
                        return Ok(shown);
                    }
                }
                return Ok(true);
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(true)
}

fn parse_right_to_left(xml: &[u8], source: &Path) -> Result<bool> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"sheetView" =>
            {
                for attribute in element.attributes().flatten() {
                    if local_name(attribute.key.as_ref()) == b"rightToLeft" {
                        let value = String::from_utf8_lossy(attribute.value.as_ref());
                        let rtl = matches!(value.trim(), "1" | "true" | "on");
                        return Ok(rtl);
                    }
                }
                return Ok(false);
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(false)
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

fn resolve_relationship_target(base_part: &str, target: &str) -> String {
    if target.starts_with('/') {
        return target.trim_start_matches('/').to_owned();
    }
    let base_dir = base_part.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let mut components: Vec<&str> = base_dir
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    components.join("/")
}

fn package_entry_names(package: &[u8], source: &Path) -> Result<Vec<String>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(source, format!("invalid XLSX package: {error}")))?;
    let mut names = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format_error(source, format!("cannot read ZIP entry: {error}")))?;
        names.push(entry.name().to_owned());
    }
    Ok(names)
}

fn parse_comments(
    package: &[u8],
    source: &Path,
    _sheets: &[SheetModel],
) -> Result<Vec<CommentModel>> {
    let workbook = entry_bytes(package, "xl/workbook.xml", source)?;
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels", source)?;
    let relationship_targets = parse_relationships(&relationships, source)?;
    let workbook_sheets = parse_workbook_sheets(&workbook, source)?;

    let mut comments = Vec::new();
    for (sheet_name, relationship_id) in workbook_sheets {
        let Some(target) = relationship_targets.get(&relationship_id) else {
            continue;
        };
        let worksheet_path = normalize_relationship_target(target);
        let rels_path = worksheet_rels_path(&worksheet_path);
        let Ok(rels_bytes) = entry_bytes(package, &rels_path, source) else {
            continue;
        };
        let sheet_rels = parse_relationship_records(&rels_bytes, source)?;
        let Some(comments_rel) = sheet_rels
            .iter()
            .find(|rel| rel.kind.ends_with("/comments"))
        else {
            continue;
        };
        let comments_path = resolve_relationship_target(&worksheet_path, &comments_rel.target);
        let Ok(comments_xml) = entry_bytes(package, &comments_path, source) else {
            continue;
        };
        let parsed = parse_comments_xml(&comments_xml, &sheet_name, source)?;
        comments.extend(parsed);
    }

    Ok(comments)
}

fn parse_comments_xml(xml: &[u8], sheet_name: &str, source: &Path) -> Result<Vec<CommentModel>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut authors = Vec::new();
    let mut comments = Vec::new();
    let mut in_authors = false;
    let mut in_author = false;
    let mut in_comment = false;
    let mut in_text = false;
    let mut in_t = false;
    let mut current_author = String::new();
    let mut current_ref = String::new();
    let mut current_author_id: Option<usize> = None;
    let mut current_text = String::new();

    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid comments XML: {error}")))?
        {
            Event::Start(element) => {
                let raw_name = element.name();
                let name = local_name(raw_name.as_ref()).to_vec();
                match name.as_slice() {
                    b"authors" => in_authors = true,
                    b"author" if in_authors => {
                        in_author = true;
                        current_author.clear();
                    }
                    b"comment" => {
                        in_comment = true;
                        current_ref.clear();
                        current_author_id = None;
                        current_text.clear();
                        for attribute in element.attributes().flatten() {
                            match local_name(attribute.key.as_ref()) {
                                b"ref" => {
                                    current_ref = String::from_utf8_lossy(attribute.value.as_ref())
                                        .into_owned()
                                        .to_ascii_uppercase();
                                }
                                b"authorId" => {
                                    current_author_id =
                                        String::from_utf8_lossy(attribute.value.as_ref())
                                            .parse()
                                            .ok();
                                }
                                _ => {}
                            }
                        }
                    }
                    b"text" if in_comment => in_text = true,
                    b"t" if in_text => in_t = true,
                    _ => {}
                }
            }
            Event::Text(text) => {
                let decoded = String::from_utf8_lossy(text.as_ref());
                let unescaped = quick_xml::escape::unescape(&decoded).map_err(|error| {
                    format_error(source, format!("invalid comments text: {error}"))
                })?;
                if in_author {
                    current_author.push_str(&unescaped);
                } else if in_t {
                    current_text.push_str(&unescaped);
                }
            }
            Event::End(element) => {
                let raw_name = element.name();
                let name = local_name(raw_name.as_ref()).to_vec();
                match name.as_slice() {
                    b"authors" => in_authors = false,
                    b"author" if in_author => {
                        in_author = false;
                        authors.push(std::mem::take(&mut current_author));
                    }
                    b"t" => in_t = false,
                    b"text" => in_text = false,
                    b"comment" if in_comment => {
                        in_comment = false;
                        if current_ref.is_empty() {
                            continue;
                        }
                        let author = current_author_id
                            .and_then(|index| authors.get(index).cloned())
                            .unwrap_or_default();
                        let text = normalize_comment_text(&current_text, &author);
                        comments.push(CommentModel {
                            element_id: ids::comment_id(sheet_name, &current_ref, SCHEMA_VERSION),
                            sheet: sheet_name.to_owned(),
                            cell: current_ref.clone(),
                            author,
                            text,
                        });
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(comments)
}

/// Strip a leading `Author:` prefix when the rich-text note includes one.
fn normalize_comment_text(raw: &str, author: &str) -> String {
    let trimmed = raw.trim_start_matches('\n').trim_end();
    if author.is_empty() {
        return trimmed.to_owned();
    }
    let prefixed = format!("{author}:");
    if let Some(rest) = trimmed.strip_prefix(&prefixed) {
        rest.trim_start_matches('\n').trim_start().to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn parse_charts(package: &[u8], source: &Path, _sheets: &[SheetModel]) -> Result<Vec<ChartModel>> {
    let chart_to_sheet = chart_sheet_map(package, source)?;
    let names = package_entry_names(package, source)?;
    let mut charts = Vec::new();
    for name in names {
        if !(name.starts_with("xl/charts/") && name.ends_with(".xml")) {
            continue;
        }
        let Ok(xml) = entry_bytes(package, &name, source) else {
            continue;
        };
        let title = parse_chart_title(&xml).unwrap_or_default();
        charts.push(ChartModel {
            element_id: ids::chart_id(&name, SCHEMA_VERSION),
            sheet: chart_to_sheet.get(&name).cloned(),
            title,
        });
    }
    charts.sort_by(|left, right| left.element_id.cmp(&right.element_id));
    Ok(charts)
}

fn parse_pictures(
    package: &[u8],
    source: &Path,
    _sheets: &[SheetModel],
) -> Result<Vec<PictureModel>> {
    let content_types = parse_content_types(package, source)?;
    let workbook = entry_bytes(package, "xl/workbook.xml", source)?;
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels", source)?;
    let relationship_targets = parse_relationships(&relationships, source)?;
    let workbook_sheets = parse_workbook_sheets(&workbook, source)?;

    let mut pictures = Vec::new();
    for (sheet_name, relationship_id) in workbook_sheets {
        let Some(target) = relationship_targets.get(&relationship_id) else {
            continue;
        };
        let worksheet_path = normalize_relationship_target(target);
        let rels_path = worksheet_rels_path(&worksheet_path);
        let Ok(rels_bytes) = entry_bytes(package, &rels_path, source) else {
            continue;
        };
        let sheet_rels = parse_relationship_records(&rels_bytes, source)?;
        for rel in sheet_rels {
            if !rel.kind.ends_with("/drawing") {
                continue;
            }
            let drawing_path = resolve_relationship_target(&worksheet_path, &rel.target);
            let Ok(drawing_xml) = entry_bytes(package, &drawing_path, source) else {
                continue;
            };
            let drawing_rels_path = worksheet_rels_path(&drawing_path);
            let Ok(drawing_rels) = entry_bytes(package, &drawing_rels_path, source) else {
                continue;
            };
            let mut image_by_rid = BTreeMap::new();
            for drawing_rel in parse_relationship_records(&drawing_rels, source)? {
                if !drawing_rel.kind.ends_with("/image") {
                    continue;
                }
                let media_path = resolve_relationship_target(&drawing_path, &drawing_rel.target);
                image_by_rid.insert(drawing_rel.id, media_path);
            }
            let parsed =
                parse_drawing_pictures(&drawing_xml, &sheet_name, &image_by_rid, &content_types)?;
            pictures.extend(parsed);
        }
    }
    pictures.sort_by(|left, right| left.element_id.cmp(&right.element_id));
    Ok(pictures)
}

fn parse_content_types(package: &[u8], source: &Path) -> Result<ContentTypesIndex> {
    let xml = entry_bytes(package, "[Content_Types].xml", source)?;
    let text = std::str::from_utf8(&xml)
        .map_err(|error| format_error(source, format!("content types are not UTF-8: {error}")))?;
    let mut defaults = BTreeMap::new();
    let mut overrides = BTreeMap::new();

    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find("<Default ") {
        let start = cursor + relative;
        let end = text[start..]
            .find('>')
            .map(|offset| start + offset)
            .ok_or_else(|| format_error(source, "unterminated Default content type"))?;
        let tag = &text[start..=end];
        if let (Some(extension), Some(content_type)) =
            (xml_attr(tag, "Extension"), xml_attr(tag, "ContentType"))
        {
            defaults.insert(extension.to_ascii_lowercase(), content_type);
        }
        cursor = end + 1;
    }

    cursor = 0;
    while let Some(relative) = text[cursor..].find("<Override ") {
        let start = cursor + relative;
        let end = text[start..]
            .find('>')
            .map(|offset| start + offset)
            .ok_or_else(|| format_error(source, "unterminated Override content type"))?;
        let tag = &text[start..=end];
        if let (Some(part_name), Some(content_type)) =
            (xml_attr(tag, "PartName"), xml_attr(tag, "ContentType"))
        {
            let normalized = part_name.trim_start_matches('/').to_owned();
            overrides.insert(normalized, content_type);
        }
        cursor = end + 1;
    }

    Ok(ContentTypesIndex {
        defaults,
        overrides,
    })
}

#[derive(Clone, Debug, Default)]
struct ContentTypesIndex {
    defaults: BTreeMap<String, String>,
    overrides: BTreeMap<String, String>,
}

impl ContentTypesIndex {
    fn content_type_for(&self, part_path: &str) -> String {
        if let Some(value) = self.overrides.get(part_path) {
            return value.clone();
        }
        let extension = part_path
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase())
            .unwrap_or_default();
        self.defaults
            .get(&extension)
            .cloned()
            .unwrap_or_else(|| "application/octet-stream".to_owned())
    }
}

fn xml_attr(tag: &str, attribute: &str) -> Option<String> {
    let needle = format!("{attribute}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

fn parse_drawing_pictures(
    xml: &[u8],
    sheet_name: &str,
    image_by_rid: &BTreeMap<String, String>,
    content_types: &ContentTypesIndex,
) -> Result<Vec<PictureModel>> {
    let text = match std::str::from_utf8(xml) {
        Ok(value) => value,
        Err(_) => return Ok(Vec::new()),
    };
    let mut pictures = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..]
        .find("<xdr:twoCellAnchor")
        .or_else(|| text[cursor..].find("<xdr:oneCellAnchor"))
    {
        let start = cursor + relative;
        let is_two = text[start..].starts_with("<xdr:twoCellAnchor");
        let close = if is_two {
            "</xdr:twoCellAnchor>"
        } else {
            "</xdr:oneCellAnchor>"
        };
        let Some(end_rel) = text[start..].find(close) else {
            break;
        };
        let end = start + end_rel + close.len();
        let anchor = &text[start..end];
        cursor = end;

        if !anchor.contains("<xdr:pic") && !anchor.contains("<xdr:pic>") {
            // Still allow pic without checking both; continue if no blip.
        }
        let Some(embed) = blip_embed_id(anchor) else {
            continue;
        };
        let Some(media_path) = image_by_rid.get(&embed) else {
            continue;
        };
        let name = cnvpr_name(anchor).unwrap_or_else(|| {
            media_path
                .rsplit('/')
                .next()
                .unwrap_or(media_path)
                .to_owned()
        });
        let from_cell = anchor_from_cell(anchor);
        pictures.push(PictureModel {
            element_id: ids::picture_id(sheet_name, media_path, SCHEMA_VERSION),
            sheet: sheet_name.to_owned(),
            name,
            content_type: content_types.content_type_for(media_path),
            from_cell,
        });
    }
    Ok(pictures)
}

fn blip_embed_id(anchor: &str) -> Option<String> {
    let markers = ["r:embed=\"", "embed=\""];
    for marker in markers {
        if let Some(start) = anchor.find(marker) {
            let value_start = start + marker.len();
            let value_end = anchor[value_start..].find('"')? + value_start;
            return Some(anchor[value_start..value_end].to_owned());
        }
    }
    None
}

fn cnvpr_name(anchor: &str) -> Option<String> {
    let start = anchor.find("<xdr:cNvPr ")?;
    let end = anchor[start..].find('>')? + start;
    let tag = &anchor[start..=end];
    xml_attr(tag, "name")
}

fn anchor_from_cell(anchor: &str) -> Option<String> {
    let from_start = anchor.find("<xdr:from>")?;
    let from_end = anchor[from_start..].find("</xdr:from>")? + from_start;
    let from = &anchor[from_start..from_end];
    let col = xml_element_u32(from, "xdr:col")?;
    let row = xml_element_u32(from, "xdr:row")?;
    Some(format!("{}{}", column_name(col as usize), row + 1))
}

fn xml_element_u32(source: &str, local: &str) -> Option<u32> {
    let open = format!("<{local}>");
    let close = format!("</{local}>");
    let start = source.find(&open)? + open.len();
    let end = source[start..].find(&close)? + start;
    source[start..end].trim().parse().ok()
}

fn chart_sheet_map(package: &[u8], source: &Path) -> Result<BTreeMap<String, String>> {
    let workbook = entry_bytes(package, "xl/workbook.xml", source)?;
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels", source)?;
    let relationship_targets = parse_relationships(&relationships, source)?;
    let workbook_sheets = parse_workbook_sheets(&workbook, source)?;

    let mut chart_to_sheet = BTreeMap::new();
    for (sheet_name, relationship_id) in workbook_sheets {
        let Some(target) = relationship_targets.get(&relationship_id) else {
            continue;
        };
        let worksheet_path = normalize_relationship_target(target);
        let rels_path = worksheet_rels_path(&worksheet_path);
        let Ok(rels_bytes) = entry_bytes(package, &rels_path, source) else {
            continue;
        };
        let sheet_rels = parse_relationship_records(&rels_bytes, source)?;
        for rel in sheet_rels {
            if !rel.kind.ends_with("/drawing") {
                continue;
            }
            let drawing_path = resolve_relationship_target(&worksheet_path, &rel.target);
            let drawing_rels_path = worksheet_rels_path(&drawing_path);
            let Ok(drawing_rels) = entry_bytes(package, &drawing_rels_path, source) else {
                continue;
            };
            let drawing_relationships = parse_relationship_records(&drawing_rels, source)?;
            for drawing_rel in drawing_relationships {
                if !drawing_rel.kind.ends_with("/chart") {
                    continue;
                }
                let chart_path = resolve_relationship_target(&drawing_path, &drawing_rel.target);
                chart_to_sheet.insert(chart_path, sheet_name.clone());
            }
        }
    }
    Ok(chart_to_sheet)
}

fn parse_chart_title(xml: &[u8]) -> Option<String> {
    let source = std::str::from_utf8(xml).ok()?;
    let title_start = source.find("<c:title")?;
    let title_end = source[title_start..].find("</c:title>")? + title_start;
    let title_body = &source[title_start..title_end];
    let mut text = String::new();
    let mut cursor = 0;
    while let Some(relative) = title_body[cursor..].find("<a:t") {
        let start = cursor + relative;
        let open_end = title_body[start..].find('>')? + start;
        let close = title_body[open_end..].find("</a:t>")? + open_end;
        let value = &title_body[open_end + 1..close];
        text.push_str(&quick_xml::escape::unescape(value).ok()?);
        cursor = close + "</a:t>".len();
    }
    Some(text)
}

fn worksheet_rels_path(part_path: &str) -> String {
    match part_path.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part_path}.rels"),
    }
}

#[derive(Clone, Debug)]
struct RelationshipRecord {
    id: String,
    kind: String,
    target: String,
}

fn parse_relationship_records(xml: &[u8], source: &Path) -> Result<Vec<RelationshipRecord>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut relationships = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(source, format!("invalid relationships XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if local_name(element.name().as_ref()) == b"Relationship" =>
            {
                let mut id = None;
                let mut kind = None;
                let mut target = None;
                for attribute in element.attributes().flatten() {
                    match local_name(attribute.key.as_ref()) {
                        b"Id" => {
                            id =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        b"Type" => {
                            kind =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        b"Target" => {
                            target =
                                Some(String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
                        }
                        _ => {}
                    }
                }
                if let (Some(id), Some(kind), Some(target)) = (id, kind, target) {
                    relationships.push(RelationshipRecord { id, kind, target });
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(relationships)
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
