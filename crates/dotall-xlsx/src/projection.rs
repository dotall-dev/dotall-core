use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;

use crate::model::{CellModel, CellValue, SheetModel, WorkbookModel};
use crate::selector::CellAddress;

pub const MAX_RENDERED_CELLS: usize = 10_000;

pub struct MarkdownRender {
    pub content: String,
    pub truncated: bool,
}

pub fn markdown_sheet(sheet: &SheetModel) -> String {
    markdown_range(
        sheet,
        CellAddress { row: 1, col: 1 },
        CellAddress {
            row: sheet.dimensions.rows.max(1),
            col: sheet.dimensions.cols.max(1),
        },
    )
    .content
}

pub fn markdown_range(sheet: &SheetModel, start: CellAddress, end: CellAddress) -> MarkdownRender {
    markdown_range_with_limit(sheet, start, end, MAX_RENDERED_CELLS)
}

pub fn markdown_range_with_limit(
    sheet: &SheetModel,
    start: CellAddress,
    end: CellAddress,
    max_cells: usize,
) -> MarkdownRender {
    let mut output = format!("## {}!{}:{}\n\n", sheet.name, address(start), address(end));
    let requested_cell_count = (u64::from(end.row) - u64::from(start.row) + 1)
        .saturating_mul(u64::from(end.col) - u64::from(start.col) + 1);
    let mut rendered_cells = sheet
        .cells
        .iter()
        .filter(|cell| {
            (start.row..=end.row).contains(&cell.row) && (start.col..=end.col).contains(&cell.col)
        })
        .take(max_cells.saturating_add(1))
        .collect::<Vec<_>>();
    let truncated = requested_cell_count > max_cells as u64 || rendered_cells.len() > max_cells;
    rendered_cells.truncate(max_cells);

    let mut columns = BTreeSet::new();
    let mut rows = BTreeMap::<u32, BTreeMap<u32, String>>::new();
    for cell in rendered_cells {
        columns.insert(cell.col);
        rows.entry(cell.row)
            .or_default()
            .insert(cell.col, display_cell(cell));
    }

    if let Some((_, header)) = rows.first_key_value() {
        let header = row_values(header, &columns);
        output.push_str(&table_row(&header));
        output.push_str(&table_row(&vec!["---".to_owned(); header.len()]));
        for (_, row) in rows.iter().skip(1) {
            output.push_str(&table_row(&row_values(row, &columns)));
        }
    }

    MarkdownRender {
        content: output,
        truncated,
    }
}

pub fn markdown_workbook(workbook: &WorkbookModel) -> String {
    let mut output = String::from("# Workbook\n\n");
    for sheet in &workbook.sheets {
        output.push_str(&markdown_sheet(sheet));
        output.push('\n');
    }
    output
}

fn row_values(row: &BTreeMap<u32, String>, columns: &BTreeSet<u32>) -> Vec<String> {
    columns
        .iter()
        .map(|column| row.get(column).cloned().unwrap_or_default())
        .collect()
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

pub fn named_ranges(workbook: &WorkbookModel) -> serde_json::Value {
    json!(
        workbook
            .named_ranges
            .iter()
            .map(|range| {
                json!({
                    "name": range.name,
                    "formula": range.formula,
                    "element_id": range.element_id,
                })
            })
            .collect::<Vec<_>>()
    )
}

pub fn merges(sheet: &SheetModel) -> serde_json::Value {
    json!({
        "sheet": sheet.name,
        "merges": sheet.merges,
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
