use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::{XlsxFormat, parse_workbook};
use rust_xlsxwriter::{Color, Workbook};
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn capabilities_advertise_set_tab_color() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source, None);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "set_tab_color"),
        "capabilities must advertise set_tab_color"
    );
}

#[test]
fn inspect_and_parse_expose_tab_color() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source, Some(Color::RGB(0x4472C4)));

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let workbook = parse_workbook(&source).expect("parse workbook");
    let inputs = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == "Inputs")
        .expect("Inputs");
    assert_eq!(inputs.tab_color.as_deref(), Some("FF4472C4"));

    let inspection = handler.inspect(&model).expect("inspect");
    assert_eq!(
        inspection.summary["sheets"][0]["tab_color"], "FF4472C4",
        "inspect must surface tab_color"
    );
}

#[test]
fn set_tab_color_patches_sheet_pr_and_leaves_other_sheet_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source, None);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_tab_color".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "color": "FF4472C4" }),
            }],
        )
        .expect("validate set_tab_color");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let sheet_xml = entry_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet_xml.contains("tabColor") && sheet_xml.contains("FF4472C4"),
        "expected tabColor rgb in sheet XML, got: {sheet_xml}"
    );
    fs::write(&source, &patched.bytes).expect("rewrite");
    let after = parse_workbook(&source).expect("reparse");
    assert_eq!(after.sheets[0].tab_color.as_deref(), Some("FF4472C4"));
    assert_eq!(edit.semantic_diff[0].change, "set_tab_color");
    assert_untouched_parts(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn set_tab_color_clear_removes_tab_color() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source, Some(Color::RGB(0xFF0000)));

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    assert_eq!(
        parse_workbook(&source).expect("parse").sheets[0]
            .tab_color
            .as_deref(),
        Some("FFFF0000")
    );
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_tab_color".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "color": null }),
            }],
        )
        .expect("validate clear");
    let patched = handler.apply_edit(&source, &edit).expect("apply clear");
    fs::write(&source, &patched.bytes).expect("rewrite");
    let after = parse_workbook(&source).expect("reparse");
    assert_eq!(after.sheets[0].tab_color, None);
    let sheet_xml = entry_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        !sheet_xml.contains("tabColor"),
        "cleared tab color must remove tabColor element"
    );
}

#[test]
fn set_tab_color_rejects_invalid_color() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source, None);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_tab_color".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "color": "not-a-color" }),
            }],
        )
        .expect_err("invalid color");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("color") || message.contains("rgb") || message.contains("hex"),
        "unexpected error: {error}"
    );
}

fn write_fixture(path: &std::path::Path, tab: Option<Color>) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("name");
    inputs.write(0, 0, "Rate").expect("label");
    if let Some(color) = tab {
        inputs.set_tab_color(color);
    }
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("name");
    revenue.write(0, 0, "Total").expect("label");
    workbook.save(path).expect("save");
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
