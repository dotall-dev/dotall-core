use serde_json::json;

use crate::model::{CellModel, CellValue, SheetModel, WorkbookModel};
use crate::selector::CellAddress;

pub fn markdown_sheet(sheet: &SheetModel) -> String {
    markdown_range(
        sheet,
        CellAddress { row: 1, col: 1 },
        CellAddress {
            row: sheet.dimensions.rows.max(1),
            col: sheet.dimensions.cols.max(1),
        },
    )
}

pub fn markdown_range(sheet: &SheetModel, start: CellAddress, end: CellAddress) -> String {
    let mut output = format!("## {}!{}:{}\n\n", sheet.name, address(start), address(end));
    let rows = (start.row..=end.row)
        .map(|row| {
            (start.col..=end.col)
                .map(|col| {
                    sheet
                        .cells
                        .iter()
                        .find(|cell| cell.row == row && cell.col == col)
                        .map_or_else(String::new, display_cell)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    if let Some(header) = rows.first() {
        output.push_str(&table_row(header));
        output.push_str(&table_row(&vec!["---".to_owned(); header.len()]));
        for row in rows.iter().skip(1) {
            output.push_str(&table_row(row));
        }
    }

    output
}

pub fn markdown_workbook(workbook: &WorkbookModel) -> String {
    let mut output = String::from("# Workbook\n\n");
    for sheet in &workbook.sheets {
        output.push_str(&markdown_sheet(sheet));
        output.push('\n');
    }
    output
}

pub fn ast_range(sheet: &SheetModel, start: CellAddress, end: CellAddress) -> serde_json::Value {
    let cells = sheet
        .cells
        .iter()
        .filter(|cell| {
            (start.row..=end.row).contains(&cell.row) && (start.col..=end.col).contains(&cell.col)
        })
        .collect::<Vec<_>>();

    json!({
        "sheet": {
            "element_id": sheet.element_id,
            "name": sheet.name,
            "index": sheet.index,
        },
        "range": format!("{}!{}:{}", sheet.name, address(start), address(end)),
        "cells": cells,
    })
}

fn table_row(cells: &[String]) -> String {
    format!("| {} |\n", cells.join(" | "))
}

fn display_cell(cell: &CellModel) -> String {
    let value = match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::String(value) => value.clone(),
        CellValue::Float(value) => {
            if value.fract() == 0.0 {
                format!("{value:.0}")
            } else {
                value.to_string()
            }
        }
        CellValue::Integer(value) => value.to_string(),
        CellValue::Boolean(value) => value.to_string(),
        CellValue::Error(value) | CellValue::Datetime(value) => value.clone(),
    };

    cell.formula
        .as_ref()
        .map_or(value.clone(), |formula| format!("{value} ({formula})"))
}

fn address(cell: CellAddress) -> String {
    format!("{}{}", column_name(cell.col), cell.row)
}

fn column_name(mut col: u32) -> String {
    let mut letters = Vec::new();
    while col > 0 {
        col -= 1;
        letters.push((b'A' + (col % 26) as u8) as char);
        col /= 26;
    }
    letters.iter().rev().collect()
}
