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
    /// Worksheet tab color as OOXML `rgb` AARRGGBB (e.g. `FF4472C4`). Absent when default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_color: Option<String>,
    pub cells: Vec<CellModel>,
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
