use std::fs;
use std::io::{Cursor, Read, Write};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::{XlsxFormat, parse_workbook};
use rust_xlsxwriter::{Format, Workbook};
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn insert_row_shifts_cells_formulas_merges_and_cross_sheet_references() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("rows.xlsx");
    write_fixture(&source);
    add_calculation_chain(&source);
    let before = fs::read(&source).expect("fixture bytes");
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "insert_row".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "at": 2, "count": 1 }),
            }],
        )
        .expect("validate insert");

    let patched = handler.apply_edit(&source, &edit).expect("apply insert");
    let reopened = parse_bytes(&patched.bytes, directory.path());
    assert_eq!(value(&reopened, "Inputs", "A3"), "2");
    assert_eq!(formula(&reopened, "Inputs", "B3"), "=A3*2");
    assert_eq!(formula(&reopened, "Summary", "A1"), "=Inputs!A3");
    assert!(entry(&patched.bytes, "xl/worksheets/sheet1.xml").contains(r#"ref="A1:B3""#));
    assert!(!zip_entries(&patched.bytes).contains_key("xl/calcChain.xml"));
    assert!(
        !entry(&patched.bytes, "[Content_Types].xml").contains(r#"PartName="/xl/calcChain.xml""#)
    );
    assert_untouched_entries_are_identical(
        &before,
        &patched.bytes,
        &[
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "[Content_Types].xml",
            "xl/calcChain.xml",
        ],
    );
}

#[test]
fn delete_row_removes_interval_and_shifts_remaining_cells() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("rows.xlsx");
    write_fixture(&source);
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "delete_row".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "at": 2, "count": 1 }),
            }],
        )
        .expect("validate delete");

    let patched = handler.apply_edit(&source, &edit).expect("apply delete");
    let reopened = parse_bytes(&patched.bytes, directory.path());
    assert_eq!(value(&reopened, "Inputs", "A2"), "3");
    assert_eq!(formula(&reopened, "Inputs", "B2"), "=A2*2");
    assert_eq!(formula(&reopened, "Summary", "A1"), "=#REF!");
}

#[test]
fn insert_column_shifts_cells_formulas_merges_and_cross_sheet_references() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("columns.xlsx");
    write_fixture(&source);
    add_calculation_chain(&source);
    let before = fs::read(&source).expect("fixture bytes");
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "insert_column".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "at": 1, "count": 1 }),
            }],
        )
        .expect("validate insert");

    let patched = handler.apply_edit(&source, &edit).expect("apply insert");
    let reopened = parse_bytes(&patched.bytes, directory.path());
    assert_eq!(value(&reopened, "Inputs", "B2"), "2");
    assert_eq!(formula(&reopened, "Inputs", "C2"), "=B2*2");
    assert_eq!(formula(&reopened, "Summary", "A1"), "=Inputs!B2");
    assert!(entry(&patched.bytes, "xl/worksheets/sheet1.xml").contains(r#"ref="B1:C1""#));
    assert!(!zip_entries(&patched.bytes).contains_key("xl/calcChain.xml"));
    assert_untouched_entries_are_identical(
        &before,
        &patched.bytes,
        &[
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "[Content_Types].xml",
            "xl/calcChain.xml",
        ],
    );
}

#[test]
fn insert_column_inside_populated_data_expands_spanning_merges() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("columns.xlsx");
    write_fixture(&source);
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "insert_column".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "at": 2, "count": 1 }),
            }],
        )
        .expect("validate insert");

    let patched = handler.apply_edit(&source, &edit).expect("apply insert");
    let reopened = parse_bytes(&patched.bytes, directory.path());
    assert_eq!(value(&reopened, "Inputs", "A2"), "2");
    assert_eq!(formula(&reopened, "Inputs", "C2"), "=A2*2");
    assert_eq!(formula(&reopened, "Summary", "A1"), "=Inputs!A2");
    assert!(entry(&patched.bytes, "xl/worksheets/sheet1.xml").contains(r#"ref="A1:C1""#));
}

#[test]
fn delete_column_removes_interval_and_shifts_remaining_cells() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("columns.xlsx");
    write_fixture(&source);
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "delete_column".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "at": 1, "count": 1 }),
            }],
        )
        .expect("validate delete");

    let patched = handler.apply_edit(&source, &edit).expect("apply delete");
    let reopened = parse_bytes(&patched.bytes, directory.path());
    assert_eq!(formula(&reopened, "Inputs", "A2"), "=#REF!*2");
    assert_eq!(formula(&reopened, "Summary", "A1"), "=#REF!");
}

#[test]
fn structural_columns_reject_zero_and_xfd_overflow_intervals() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("columns.xlsx");
    write_fixture(&source);
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");

    for payload in [
        serde_json::json!({ "sheet": "Inputs", "at": 2, "count": 0 }),
        serde_json::json!({ "sheet": "Inputs", "at": 16_384, "count": 2 }),
    ] {
        let error = handler
            .validate_edit_with_source(
                &source,
                &model,
                &[SemanticOperation {
                    kind: "insert_column".into(),
                    payload,
                }],
            )
            .expect_err("invalid structural interval");
        assert!(
            error.to_string().contains("positive integer `count`")
                || error.to_string().contains("must not exceed 16384"),
            "unexpected error: {error}"
        );
    }

    handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "insert_column".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "at": 16_384, "count": 1 }),
            }],
        )
        .expect("XFD insertion is within the Excel column limit");
}

#[test]
fn structural_rows_reject_zero_and_out_of_range_intervals() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("rows.xlsx");
    write_fixture(&source);
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");

    for payload in [
        serde_json::json!({ "sheet": "Inputs", "at": 2, "count": 0 }),
        serde_json::json!({ "sheet": "Inputs", "at": 1_048_576, "count": 2 }),
    ] {
        let error = handler
            .validate_edit_with_source(
                &source,
                &model,
                &[SemanticOperation {
                    kind: "insert_row".into(),
                    payload,
                }],
            )
            .expect_err("invalid structural interval");
        assert!(
            error.to_string().contains("positive integer `count`")
                || error.to_string().contains("must not exceed 1048576"),
            "unexpected error: {error}"
        );
    }
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    inputs.write_number(0, 0, 1).expect("A1");
    inputs.write_number(1, 0, 2).expect("A2");
    inputs.write_formula(1, 1, "=A2*2").expect("B2");
    inputs.write_number(2, 0, 3).expect("A3");
    inputs.write_formula(2, 1, "=A3*2").expect("B3");
    inputs
        .merge_range(0, 0, 0, 1, "header", &Format::new())
        .expect("merge");
    let summary = workbook.add_worksheet().set_name("Summary").expect("sheet");
    summary
        .write_formula(0, 0, "=Inputs!A2")
        .expect("summary formula");
    workbook.save(path).expect("save fixture");
}

fn add_calculation_chain(path: &std::path::Path) {
    let original = fs::read(path).expect("fixture bytes");
    let mut archive = ZipArchive::new(Cursor::new(original)).expect("open fixture ZIP");
    let output = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(output);
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("fixture entry");
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read fixture entry");
        if name == "[Content_Types].xml" {
            let xml = String::from_utf8(bytes).expect("content types XML");
            bytes = xml
                .replace(
                    "</Types>",
                    r#"<Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/></Types>"#,
                )
                .into_bytes();
        }
        writer
            .start_file(name, options)
            .expect("copy fixture entry");
        writer.write_all(&bytes).expect("write fixture entry");
    }
    writer
        .start_file("xl/calcChain.xml", options)
        .expect("start calc chain");
    writer
        .write_all(
            br#"<calcChain xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><c r="B2" i="1"/></calcChain>"#,
        )
        .expect("write calc chain");
    fs::write(
        path,
        writer.finish().expect("finish fixture ZIP").into_inner(),
    )
    .expect("save fixture");
}

fn parse_bytes(bytes: &[u8], directory: &std::path::Path) -> dotall_xlsx::WorkbookModel {
    let path = directory.join("patched.xlsx");
    fs::write(&path, bytes).expect("write patched workbook");
    parse_workbook(&path).expect("reopen patched workbook")
}

fn value(model: &dotall_xlsx::WorkbookModel, sheet: &str, address: &str) -> String {
    let cell = model
        .sheets
        .iter()
        .find(|candidate| candidate.name == sheet)
        .and_then(|sheet| sheet.cells.iter().find(|cell| cell.address == address))
        .expect("cell");
    dotall_xlsx::edits::format_cell_value(&cell.value).expect("value")
}

fn formula(model: &dotall_xlsx::WorkbookModel, sheet: &str, address: &str) -> String {
    model
        .sheets
        .iter()
        .find(|candidate| candidate.name == sheet)
        .and_then(|sheet| sheet.cells.iter().find(|cell| cell.address == address))
        .and_then(|cell| cell.formula.clone())
        .unwrap_or_else(|| panic!("formula at {sheet}!{address}"))
}

fn entry(bytes: &[u8], name: &str) -> String {
    String::from_utf8(zip_entries(bytes)[name].clone()).expect("XML")
}

fn zip_entries(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open ZIP");
    let mut entries = std::collections::BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("entry");
        let mut data = Vec::new();
        file.read_to_end(&mut data).expect("read");
        entries.insert(file.name().to_owned(), data);
    }
    entries
}

fn assert_untouched_entries_are_identical(before: &[u8], after: &[u8], patched: &[&str]) {
    for (name, bytes) in zip_entries(before) {
        if patched.contains(&name.as_str()) || is_calc_invalidation_part(&name) {
            continue;
        }
        assert_eq!(
            zip_entries(after)[&name],
            bytes,
            "changed untouched part {name}"
        );
    }
}

fn is_calc_invalidation_part(name: &str) -> bool {
    name == "xl/workbook.xml"
        || name.starts_with("xl/worksheets/")
        || name.starts_with("xl/charts/")
}
