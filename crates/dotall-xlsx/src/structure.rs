use serde::Serialize;

use crate::model::{CellValue, WorkbookModel};

#[derive(Debug, Serialize)]
pub struct Structure {
    pub sheets: Vec<SheetStructure>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SheetStructure {
    pub sheet_element_id: String,
    pub sheet_name: String,
    pub header_row: Option<u32>,
    pub confidence: f32,
}

/// Finds a likely header in the first populated row of each sheet.
pub fn analyze(workbook: &WorkbookModel) -> Structure {
    let mut warnings = Vec::new();
    let sheets = workbook
        .sheets
        .iter()
        .map(|sheet| {
            let first_row = sheet.cells.iter().map(|cell| cell.row).min();
            let header_cells = first_row.map_or_else(Vec::new, |row| {
                sheet.cells.iter().filter(|cell| cell.row == row).collect()
            });
            let string_count = header_cells
                .iter()
                .filter(|cell| matches!(&cell.value, CellValue::String(value) if !value.trim().is_empty()))
                .count();
            let confidence = if header_cells.is_empty() {
                0.0
            } else {
                string_count as f32 / header_cells.len() as f32
            };
            let header_row = (confidence >= 0.5).then_some(first_row.unwrap_or_default());

            if header_row.is_none() && first_row.is_some() {
                warnings.push(format!("header detection low confidence on `{}`", sheet.name));
            }

            SheetStructure {
                sheet_element_id: sheet.element_id.clone(),
                sheet_name: sheet.name.clone(),
                header_row,
                confidence,
            }
        })
        .collect();

    Structure { sheets, warnings }
}
