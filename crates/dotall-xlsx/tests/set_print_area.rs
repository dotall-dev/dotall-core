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
fn capabilities_advertise_set_print_area() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "set_print_area"),
        "capabilities must advertise set_print_area"
    );
}

#[test]
fn set_print_area_patches_workbook_and_leaves_worksheets_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_print_area".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "range": "A1:B3" }),
            }],
        )
        .expect("validate set_print_area");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let workbook_xml = entry_xml(&patched.bytes, "xl/workbook.xml");
    assert!(
        workbook_xml.contains("_xlnm.Print_Area") && workbook_xml.contains("Inputs!$A$1:$B$3"),
        "expected Print_Area definedName, got: {workbook_xml}"
    );
    fs::write(&source, &patched.bytes).expect("rewrite");
    let after = parse_workbook(&source).expect("reparse");
    assert_eq!(after.sheets[0].print_area.as_deref(), Some("A1:B3"));
    assert_eq!(edit.semantic_diff[0].change, "set_print_area");

    let inspection = handler
        .inspect(&handler.parse(&source).expect("parse after"))
        .expect("inspect");
    assert_eq!(inspection.summary["sheets"][0]["print_area"], "A1:B3");
    assert_untouched_parts(&before, &patched.bytes, &["xl/workbook.xml"]);
}

#[test]
fn set_print_area_clear_removes_print_area() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture_with_print_area(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    assert_eq!(
        parse_workbook(&source).expect("parse").sheets[0]
            .print_area
            .as_deref(),
        Some("A1:B2")
    );
    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_print_area".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "range": null }),
            }],
        )
        .expect("validate clear");
    let cleared = handler.apply_edit(&source, &clear).expect("apply clear");
    fs::write(&source, &cleared.bytes).expect("rewrite");
    let after = parse_workbook(&source).expect("reparse");
    assert_eq!(after.sheets[0].print_area, None);
    let workbook_xml = entry_xml(&cleared.bytes, "xl/workbook.xml");
    assert!(
        !workbook_xml.contains("_xlnm.Print_Area"),
        "cleared print area must remove Print_Area definedName"
    );
}

#[test]
fn set_print_area_rejects_invalid_range() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_print_area".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "range": "not-a-range" }),
            }],
        )
        .expect_err("invalid range");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("range") || message.contains("invalid"),
        "unexpected error: {error}"
    );
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    inputs.write_string(0, 0, "A").expect("write");
    inputs.write_string(0, 1, "B").expect("write");
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("sheet");
    revenue.write_string(0, 0, "X").expect("write");
    workbook.save(path).expect("save");
}

fn write_fixture_with_print_area(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    inputs.write_string(0, 0, "A").expect("write");
    inputs.write_string(0, 1, "B").expect("write");
    inputs.set_print_area(0, 0, 1, 1).expect("print area A1:B2");
    workbook.add_worksheet().set_name("Revenue").expect("sheet");
    workbook.save(path).expect("save");
}

fn entry_xml(package: &[u8], name: &str) -> String {
    String::from_utf8(zip_entries(package)[name].clone()).expect("utf-8")
}

fn zip_entries(package: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package)).expect("zip");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("entry");
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read");
        entries.insert(name, bytes);
    }
    entries
}

fn assert_untouched_parts(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);
    for (name, before_bytes) in &before_entries {
        if patched.contains(&name.as_str()) {
            continue;
        }
        let after_bytes = after_entries
            .get(name)
            .unwrap_or_else(|| panic!("entry retained: {name}"));
        assert_eq!(before_bytes, after_bytes, "bytes changed for {name}");
    }
}
