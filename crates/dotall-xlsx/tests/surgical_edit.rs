use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::{XlsxFormat, parse_workbook};
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn patches_a_cell_value_without_changing_other_zip_entries() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A1",
                    "value": 42,
                }),
            }],
        )
        .expect("validate value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply value edit");
    let after_model = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(cell_value(&after_model, "Inputs", "A1"), "42");
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn patches_a_formula_without_changing_other_worksheet() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_formula".into(),
                payload: serde_json::json!({
                    "sheet": "Summary",
                    "address": "B1",
                    "formula": "Inputs!A1*2",
                }),
            }],
        )
        .expect("validate formula edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply formula edit");
    let after_model = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(formula(&after_model, "Summary", "B1"), "=Inputs!A1*2");
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet2.xml"]);
}

#[test]
fn adds_a_sparse_cell_inside_a_valid_worksheet_row() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B2",
                    "value": "created",
                }),
            }],
        )
        .expect("validate sparse value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply sparse value edit");
    let after_model = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(cell_value(&after_model, "Inputs", "B2"), "created");
    assert!(
        worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml")
            .contains(r#"<row r="2"><c r="B2""#),
        "new cells must be nested in their worksheet row"
    );
}

#[test]
fn adds_a_cell_to_an_existing_worksheet_row() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B1",
                    "value": "adjacent",
                }),
            }],
        )
        .expect("validate adjacent value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply adjacent value edit");

    assert_eq!(
        cell_value(
            &parse_workbook_bytes(&patched.bytes, directory.path()),
            "Inputs",
            "B1"
        ),
        "adjacent"
    );
    let xml = worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert_eq!(xml.matches(r#"<row r="1""#).count(), 1);
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("sheet name");
    inputs.write_number(0, 0, 1).expect("input value");
    let summary = workbook
        .add_worksheet()
        .set_name("Summary")
        .expect("sheet name");
    summary
        .write_formula(0, 1, "=Inputs!A1")
        .expect("summary formula");
    workbook.save(path).expect("write fixture");
}

fn parse_workbook_bytes(bytes: &[u8], directory: &std::path::Path) -> dotall_xlsx::WorkbookModel {
    let path = directory.join("patched.xlsx");
    fs::write(&path, bytes).expect("write patched workbook");
    parse_workbook(&path).expect("parse patched workbook")
}

fn cell_value(workbook: &dotall_xlsx::WorkbookModel, sheet: &str, address: &str) -> String {
    let cell = workbook
        .sheets
        .iter()
        .find(|candidate| candidate.name == sheet)
        .and_then(|candidate| candidate.cells.iter().find(|cell| cell.address == address))
        .expect("cell");
    dotall_xlsx::edits::format_cell_value(&cell.value).expect("cell value")
}

fn formula(workbook: &dotall_xlsx::WorkbookModel, sheet: &str, address: &str) -> String {
    workbook
        .sheets
        .iter()
        .find(|candidate| candidate.name == sheet)
        .and_then(|candidate| candidate.cells.iter().find(|cell| cell.address == address))
        .and_then(|cell| cell.formula.clone())
        .expect("formula")
}

fn assert_untouched_entries_are_identical(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);

    assert_eq!(
        before_entries.len(),
        after_entries.len(),
        "ZIP entry count changed"
    );
    for (name, (before_crc, before_bytes)) in before_entries {
        let (after_crc, after_bytes) = after_entries.get(&name).expect("entry retained");
        if !patched.contains(&name.as_str()) {
            assert_eq!(before_crc, *after_crc, "CRC changed for {name}");
            assert_eq!(before_bytes, *after_bytes, "bytes changed for {name}");
        }
    }
}

fn zip_entries(bytes: &[u8]) -> BTreeMap<String, (u32, Vec<u8>)> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open ZIP");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("ZIP entry");
        let mut contents = Vec::new();
        entry.read_to_end(&mut contents).expect("read ZIP entry");
        entries.insert(entry.name().to_owned(), (entry.crc32(), contents));
    }
    entries
}

fn worksheet_xml(bytes: &[u8], name: &str) -> String {
    let entries = zip_entries(bytes);
    String::from_utf8(entries[name].1.clone()).expect("worksheet XML")
}
