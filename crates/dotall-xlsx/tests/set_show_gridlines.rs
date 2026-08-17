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
fn capabilities_advertise_set_show_gridlines() {
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
            .any(|cap| cap.operation == "set_show_gridlines"),
        "capabilities must advertise set_show_gridlines"
    );
    assert_eq!(inspection.summary["sheets"][1]["show_gridlines"], true);
}

#[test]
fn hide_gridlines_writes_show_grid_lines_zero_and_leaves_others_byte_identical() {
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
                kind: "set_show_gridlines".into(),
                payload: serde_json::json!({
                    "sheet": "Revenue",
                    "show": false
                }),
            }],
        )
        .expect("validate set_show_gridlines");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let sheet_xml = entry_xml(&patched.bytes, "xl/worksheets/sheet2.xml");
    assert!(
        sheet_xml.contains(r#"showGridLines="0""#),
        "expected sheetView showGridLines=0, got: {sheet_xml}"
    );
    fs::write(&source, &patched.bytes).expect("rewrite");
    let after = parse_workbook(&source).expect("reparse");
    assert!(!after.sheets[1].show_gridlines);
    assert_eq!(edit.semantic_diff[0].change, "set_show_gridlines");

    let inspection = handler
        .inspect(&handler.parse(&source).expect("parse after"))
        .expect("inspect");
    assert_eq!(inspection.summary["sheets"][1]["show_gridlines"], false);
    assert_untouched_parts(&before, &patched.bytes, &["xl/worksheets/sheet2.xml"]);
}

#[test]
fn show_gridlines_removes_show_grid_lines_attribute() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_hidden_gridlines_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    assert!(!parse_workbook(&source).expect("parse hidden").sheets[1].show_gridlines);

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_show_gridlines".into(),
                payload: serde_json::json!({
                    "sheet": "Revenue",
                    "show": true
                }),
            }],
        )
        .expect("validate set_show_gridlines show");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let sheet_xml = entry_xml(&patched.bytes, "xl/worksheets/sheet2.xml");
    assert!(
        !sheet_xml.contains("showGridLines="),
        "show must remove showGridLines attribute, got: {sheet_xml}"
    );
    fs::write(&source, &patched.bytes).expect("rewrite");
    let after = parse_workbook(&source).expect("reparse");
    assert!(after.sheets[1].show_gridlines);

    let inspection = handler
        .inspect(&handler.parse(&source).expect("parse after"))
        .expect("inspect");
    assert_eq!(inspection.summary["sheets"][1]["show_gridlines"], true);
}

#[test]
fn set_show_gridlines_preserves_freeze_pane_children_and_zoom() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture_with_freeze_and_zoom(&source);
    let before = fs::read(&source).expect("fixture bytes");
    let before_xml = entry_xml(&before, "xl/worksheets/sheet1.xml");
    assert!(
        before_xml.contains("<pane") || before_xml.contains("<x:pane"),
        "fixture must include a freeze pane, got: {before_xml}"
    );
    assert!(
        before_xml.contains(r#"zoomScale="75""#),
        "fixture must include zoomScale=75, got: {before_xml}"
    );

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_show_gridlines".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "show": false
                }),
            }],
        )
        .expect("validate set_show_gridlines");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let sheet_xml = entry_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet_xml.contains(r#"showGridLines="0""#),
        "expected sheetView showGridLines=0, got: {sheet_xml}"
    );
    assert!(
        sheet_xml.contains("<pane") || sheet_xml.contains("<x:pane"),
        "freeze pane child must survive gridlines edit, got: {sheet_xml}"
    );
    assert!(
        sheet_xml.contains(r#"zoomScale="75""#),
        "zoomScale must survive gridlines edit, got: {sheet_xml}"
    );
    assert_untouched_parts(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn set_show_gridlines_rejects_missing_sheet() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_show_gridlines".into(),
                payload: serde_json::json!({
                    "sheet": "Missing",
                    "show": false
                }),
            }],
        )
        .expect_err("missing sheet must be rejected");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("unknown sheet") || message.contains("missing"),
        "unexpected error for missing sheet: {error}"
    );
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    inputs.write_string(0, 0, "A").expect("write");
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("sheet");
    revenue.write_string(0, 0, "X").expect("write");
    workbook.save(path).expect("save");
}

fn write_hidden_gridlines_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    inputs.write_string(0, 0, "A").expect("write");
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("sheet");
    revenue.write_string(0, 0, "X").expect("write");
    revenue.set_screen_gridlines(false);
    workbook.save(path).expect("save");
}

fn write_fixture_with_freeze_and_zoom(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    inputs.write_string(0, 0, "A").expect("write");
    inputs.set_freeze_panes(1, 1).expect("seed freeze panes B2");
    inputs.set_zoom(75);
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("sheet");
    revenue.write_string(0, 0, "X").expect("write");
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
