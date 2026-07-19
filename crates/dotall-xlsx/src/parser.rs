use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek};
use std::path::Path;

use calamine::{Data, Reader, open_workbook_auto};
use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;
use crate::ids;
use crate::model::{
    CellModel, CellValue, NamedRange, SCHEMA_VERSION, SheetDimensions, SheetModel, UnmodeledMap,
    WorkbookModel, column_name,
};

/// Parses an XLSX workbook into the canonical `xlsx.workbook` v1 model.
///
/// Calamine does not expose worksheet merges or number-format/style tables through
/// its public API, so v1 leaves those collections empty until an OOXML reader is
/// introduced for those preserve-only details.
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

    let sheets = workbook
        .sheet_names()
        .into_iter()
        .enumerate()
        .map(|(index, name)| parse_sheet(&mut workbook, source, name, index as u32))
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
        merges: Vec::new(),
        cells,
    })
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
        Data::DateTime(value) => CellValue::Datetime(value.to_string()),
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
