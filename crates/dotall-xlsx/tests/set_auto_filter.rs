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
fn capabilities_advertise_set_auto_filter() {
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
            .any(|cap| cap.operation == "set_auto_filter"),
        "capabilities must advertise set_auto_filter"
    );
}

#[test]
fn set_auto_filter_patches_sheet_and_leaves_other_sheet_byte_identical() {
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
                kind: "set_auto_filter".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "range": "A1:B3" }),
            }],
        )
        .expect("validate set_auto_filter");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let sheet_xml = entry_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet_xml.contains(r#"autoFilter ref="A1:B3""#)
            || sheet_xml.contains("autoFilter ref=\"A1:B3\""),
        "expected autoFilter ref in sheet XML, got: {sheet_xml}"
    );
    fs::write(&source, &patched.bytes).expect("rewrite");
    let after = parse_workbook(&source).expect("reparse");
    assert_eq!(after.sheets[0].auto_filter.as_deref(), Some("A1:B3"));
    assert_eq!(edit.semantic_diff[0].change, "set_auto_filter");

    let inspection = handler
        .inspect(&handler.parse(&source).expect("parse after"))
        .expect("inspect");
    assert_eq!(inspection.summary["sheets"][0]["auto_filter"], "A1:B3");
    assert_untouched_parts(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn set_auto_filter_clear_removes_auto_filter() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let set = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_auto_filter".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "range": "A1:B2" }),
            }],
        )
        .expect("validate set");
    let patched = handler.apply_edit(&source, &set).expect("apply set");
    fs::write(&source, &patched.bytes).expect("rewrite");

    let model = handler.parse(&source).expect("parse");
    assert_eq!(
        parse_workbook(&source).expect("parse").sheets[0]
            .auto_filter
            .as_deref(),
        Some("A1:B2")
    );
    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_auto_filter".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "range": null }),
            }],
        )
        .expect("validate clear");
    let cleared = handler.apply_edit(&source, &clear).expect("apply clear");
    fs::write(&source, &cleared.bytes).expect("rewrite");
    let after = parse_workbook(&source).expect("reparse");
    assert_eq!(after.sheets[0].auto_filter, None);
    let sheet_xml = entry_xml(&cleared.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        !sheet_xml.contains("autoFilter"),
        "cleared auto filter must remove autoFilter element"
    );
}

#[test]
fn set_auto_filter_rejects_invalid_range() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_auto_filter".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "range": "not-a-range" }),
            }],
        )
        .expect_err("invalid range");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("range") || message.contains("a1"),
        "unexpected error: {error}"
    );
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").unwrap();
    inputs.write_string(0, 0, "Metric").unwrap();
    inputs.write_string(0, 1, "Value").unwrap();
    inputs.write_string(1, 0, "ARR").unwrap();
    inputs.write_number(1, 1, 100.0).unwrap();
    let _summary = workbook.add_worksheet().set_name("Summary").unwrap();
    workbook.save(path).expect("save fixture");
}

fn entry_xml(package: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(package)).expect("zip");
    let mut entry = archive.by_name(name).expect("entry");
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).expect("read");
    String::from_utf8(bytes).expect("utf8")
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
