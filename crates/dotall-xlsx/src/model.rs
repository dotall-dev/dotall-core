use serde::{Deserialize, Serialize};

pub const SCHEMA_ID: &str = "xlsx.workbook";
pub const SCHEMA_VERSION: u32 = 1;

/// The canonical `xlsx.workbook` v1 representation of an XLSX workbook.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkbookModel {
    pub workbook_id: String,
    pub sheets: Vec<SheetModel>,
    pub named_ranges: Vec<NamedRange>,
    pub style_table: Vec<StyleEntry>,
    pub unmodeled: UnmodeledMap,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SheetModel {
    pub element_id: String,
    pub name: String,
    pub index: u32,
    pub dimensions: SheetDimensions,
    pub merges: Vec<String>,
    /// Openpyxl-style freeze cell (`B2` freezes row 1 + col A). Absent when unfrozen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze_panes: Option<String>,
    /// Worksheet view zoom percent (10–400) from `sheetView/@zoomScale`. Absent when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<u32>,
    /// Worksheet view gridlines. `false` when `sheetView/@showGridLines="0"`; default shown is `true`.
    #[serde(default = "default_show_gridlines")]
    pub show_gridlines: bool,
    /// Worksheet tab color as OOXML `rgb` AARRGGBB (e.g. `FF4472C4`). Absent when default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_color: Option<String>,
    /// AutoFilter range as A1 (`A1:D10`). Absent when no auto filter is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_filter: Option<String>,
    /// Print area as A1 (`A1:D10`). Absent when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub print_area: Option<String>,
    /// Print titles (rows/cols to repeat). Absent when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub print_titles: Option<PrintTitles>,
    /// Page orientation (`portrait` / `landscape`). Absent when unset in OOXML.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_orientation: Option<String>,
    /// Print paper size from `pageSetup/@paperSize` (e.g. 1=Letter, 9=A4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_size: Option<u32>,
    /// Print scale percent (10–400) from `pageSetup/@scale`. Absent when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub print_scale: Option<u32>,
    /// Fit-to-page widths/heights from `pageSetup` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit_to_page: Option<FitToPage>,
    /// Print centering from `printOptions` when either axis is centered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center_on_page: Option<CenterOnPage>,
    /// Print page margins in inches. Absent when no `pageMargins` element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_margins: Option<PageMargins>,
    /// Print header/footer (`oddHeader` / `oddFooter`). Absent when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_footer: Option<HeaderFooter>,
    pub cells: Vec<CellModel>,
}

fn default_show_gridlines() -> bool {
    true
}

/// Worksheet print header/footer (`headerFooter` oddHeader / oddFooter).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct HeaderFooter {
    /// Odd-page header text, including Excel codes such as `&C`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// Odd-page footer text, including Excel codes such as `&P`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<String>,
}

/// Worksheet print centering (`printOptions` horizontalCentered / verticalCentered).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct CenterOnPage {
    pub horizontal: bool,
    pub vertical: bool,
}

/// Worksheet fit-to-page (`pageSetup` fitToWidth / fitToHeight).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct FitToPage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// Worksheet print margins (`pageMargins`) in inches.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PageMargins {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<f64>,
}

/// Worksheet print titles (`_xlnm.Print_Titles`): repeat rows and/or columns when printing.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct PrintTitles {
    /// Row span like `1:1` (1-based inclusive). Absent when only columns repeat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<String>,
    /// Column span like `A:B`. Absent when only rows repeat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SheetDimensions {
    pub rows: u32,
    pub cols: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CellModel {
    pub element_id: String,
    pub address: String,
    pub row: u32,
    pub col: u32,
    pub value: CellValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_format: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CellValue {
    Empty,
    String(String),
    Float(f64),
    Integer(i64),
    Boolean(bool),
    Error(String),
    Datetime(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NamedRange {
    pub element_id: String,
    pub name: String,
    pub formula: String,
}

/// A minimal, stable style-table entry for v1.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StyleEntry {
    pub style_id: String,
}

/// Preserve-only OOXML feature classes that are intentionally outside v1.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct UnmodeledMap {
    pub charts: PreservationStatus,
    pub pivots: PreservationStatus,
    pub vba: PreservationStatus,
    pub other_ooxml_parts: PreservationStatus,
}

impl Default for UnmodeledMap {
    fn default() -> Self {
        Self {
            charts: PreservationStatus::Preserved,
            pivots: PreservationStatus::Preserved,
            vba: PreservationStatus::Preserved,
            other_ooxml_parts: PreservationStatus::Preserved,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreservationStatus {
    Preserved,
}

/// Converts a zero-based column index to its Excel column name.
pub fn column_name(mut zero_based: usize) -> String {
    let mut letters = Vec::new();

    loop {
        letters.push((b'A' + (zero_based % 26) as u8) as char);

        if zero_based < 26 {
            break;
        }

        zero_based = zero_based / 26 - 1;
    }

    letters.iter().rev().collect()
}
