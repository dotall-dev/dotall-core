use std::fs;
use std::io::{Cursor, Read, Write};

use calamine::Reader;
use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::{XlsxFormat, parse_workbook};
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;
use zip::{ZipArchive, ZipWriter};

#[test]
fn adds_sheet_after_requested_sheet_and_preserves_unrelated_entries() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");

    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "add_sheet".into(),
                payload: serde_json::json!({ "name": "Data", "after": "Inputs" }),
            }],
        )
        .expect("validate add sheet");
    let patched = handler.apply_edit(&source, &edit).expect("apply add sheet");
    let reopened = parse_bytes(&patched.bytes, directory.path());

    assert_eq!(
        reopened
            .sheets
            .iter()
            .map(|sheet| sheet.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Inputs", "Data", "Summary"]
    );
    assert!(zip_entries(&patched.bytes).contains_key("xl/worksheets/sheet3.xml"));
    assert_untouched_entries_are_identical(
        &before,
        &patched.bytes,
        &[
            "xl/workbook.xml",
            "xl/_rels/workbook.xml.rels",
            "[Content_Types].xml",
            "docProps/app.xml",
        ],
    );
}

#[test]
fn rejects_invalid_or_case_colliding_sheet_names() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");

    for name in [
        "",
        "'quoted",
        "quoted'",
        "bad/name",
        &"x".repeat(32),
        "inputs",
    ] {
        let error = handler
            .validate_edit_with_source(
                &source,
                &model,
                &[SemanticOperation {
                    kind: "add_sheet".into(),
                    payload: serde_json::json!({ "name": name }),
                }],
            )
            .expect_err("invalid sheet name");
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn renames_sheet_and_rewrites_qualified_formula_references() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");

    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "rename_sheet".into(),
                payload: serde_json::json!({ "from": "Inputs", "to": "O'Brien Data" }),
            }],
        )
        .expect("validate rename");
    let patched = handler.apply_edit(&source, &edit).expect("apply rename");
    let reopened = parse_bytes(&patched.bytes, directory.path());

    assert!(
        reopened
            .sheets
            .iter()
            .any(|sheet| sheet.name == "O'Brien Data")
    );
    assert_eq!(formula(&reopened, "Summary", "A1"), "='O''Brien Data'!A1");
    assert_calamine_reopens(&patched.bytes, directory.path());
    assert_untouched_entries_are_identical(
        &before,
        &patched.bytes,
        &[
            "xl/workbook.xml",
            "xl/worksheets/sheet2.xml",
            "docProps/app.xml",
        ],
    );
}

#[test]
fn rejects_rename_when_a_chart_reference_would_need_rewriting() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    append_zip_entry(
        &source,
        "xl/charts/chart1.xml",
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:f>Inputs!$A$1</c:f></c:chartSpace>"#,
    );
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");

    let error = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "rename_sheet".into(),
                payload: serde_json::json!({ "from": "Inputs", "to": "Data" }),
            }],
        )
        .expect_err("chart reference must reject rename");
    assert!(error.to_string().contains("chart1.xml"));
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("sheet name")
        .write_number(0, 0, 1)
        .expect("input value");
    workbook
        .add_worksheet()
        .set_name("Summary")
        .expect("sheet name")
        .write_formula(0, 0, "=Inputs!A1")
        .expect("formula");
    workbook.save(path).expect("write fixture");
}

fn parse_bytes(bytes: &[u8], directory: &std::path::Path) -> dotall_xlsx::WorkbookModel {
    let path = directory.join("patched.xlsx");
    fs::write(&path, bytes).expect("write patched workbook");
    parse_workbook(&path).expect("parse patched workbook")
}

fn formula(workbook: &dotall_xlsx::WorkbookModel, sheet: &str, address: &str) -> String {
    workbook
        .sheets
        .iter()
        .find(|candidate| candidate.name == sheet)
        .and_then(|sheet| sheet.cells.iter().find(|cell| cell.address == address))
        .and_then(|cell| cell.formula.clone())
        .expect("formula")
}

fn assert_calamine_reopens(bytes: &[u8], directory: &std::path::Path) {
    let path = directory.join("calamine.xlsx");
    fs::write(&path, bytes).expect("write calamine workbook");
    let workbook = calamine::open_workbook_auto(path).expect("calamine reopen");
    assert!(!workbook.sheet_names().is_empty());
}

fn append_zip_entry(path: &std::path::Path, name: &str, contents: &str) {
    let source = fs::read(path).expect("fixture bytes");
    let mut archive = ZipArchive::new(Cursor::new(source)).expect("open fixture ZIP");
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let entry = archive.by_index(index).expect("ZIP entry");
        output.raw_copy_file(entry).expect("copy ZIP entry");
    }
    output
        .start_file(name, zip::write::SimpleFileOptions::default())
        .expect("start chart entry");
    output
        .write_all(contents.as_bytes())
        .expect("write chart entry");
    fs::write(path, output.finish().expect("finish ZIP").into_inner()).expect("write fixture");
}

fn zip_entries(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open ZIP");
    let mut entries = std::collections::BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("ZIP entry");
        let mut contents = Vec::new();
        entry.read_to_end(&mut contents).expect("read ZIP entry");
        entries.insert(entry.name().to_owned(), contents);
    }
    entries
}

fn assert_untouched_entries_are_identical(before: &[u8], after: &[u8], patched: &[&str]) {
    for (name, bytes) in zip_entries(before) {
        if !patched.contains(&name.as_str()) {
            assert_eq!(
                zip_entries(after)[&name],
                bytes,
                "changed untouched part {name}"
            );
        }
    }
}
