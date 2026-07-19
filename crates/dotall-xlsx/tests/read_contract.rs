use dotall_xlsx::{CellValue, parse_workbook};
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

#[test]
fn parses_sparse_cells_formulas_and_deterministic_hybrid_ids() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("financials.xlsx");

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Revenue").expect("sheet name");
    worksheet.write_string(0, 0, "Month").expect("header");
    worksheet.write_number(1, 0, 100.0).expect("value");
    worksheet.write_formula(1, 1, "=A2*1.1").expect("formula");
    workbook.save(&path).expect("workbook fixture");

    let parsed = parse_workbook(&path).expect("parse workbook");
    let reparsed = parse_workbook(&path).expect("reparse workbook");

    assert_eq!(parsed.workbook_id, reparsed.workbook_id);
    assert_eq!(parsed.sheets.len(), 1);

    let sheet = &parsed.sheets[0];
    assert_eq!(sheet.name, "Revenue");
    assert_eq!(sheet.cells.len(), 3, "empty cells remain sparse");

    let formula_cell = sheet
        .cells
        .iter()
        .find(|cell| cell.address == "B2")
        .expect("formula cell");
    assert_eq!(formula_cell.row, 2);
    assert_eq!(formula_cell.col, 2);
    assert_eq!(formula_cell.formula.as_deref(), Some("=A2*1.1"));
    assert!(formula_cell.element_id.starts_with("c_"));
    assert_ne!(formula_cell.element_id, formula_cell.address);
    assert_eq!(formula_cell.value, CellValue::Float(0.0));

    let reparsed_formula_cell = reparsed.sheets[0]
        .cells
        .iter()
        .find(|cell| cell.address == "B2")
        .expect("reparsed formula cell");
    assert_eq!(formula_cell.element_id, reparsed_formula_cell.element_id);
}

#[test]
fn dimensions_cover_independent_farthest_row_and_column() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("dimensions.xlsx");

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet
        .write_string(0, 25, "far column")
        .expect("far column");
    worksheet.write_string(9, 0, "far row").expect("far row");
    workbook.save(&path).expect("workbook fixture");

    let parsed = parse_workbook(&path).expect("parse workbook");

    assert_eq!(parsed.sheets[0].dimensions.rows, 10);
    assert_eq!(parsed.sheets[0].dimensions.cols, 26);
}
