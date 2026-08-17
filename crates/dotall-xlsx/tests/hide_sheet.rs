use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::XlsxFormat;
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn capabilities_advertise_hide_sheet() {
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
            .any(|cap| cap.operation == "hide_sheet"),
        "capabilities must advertise hide_sheet"
    );
}

#[test]
fn hide_sheet_sets_state_hidden_and_leaves_worksheets_byte_identical() {
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
                kind: "hide_sheet".into(),
                payload: serde_json::json!({ "sheet": "Revenue", "hidden": true }),
            }],
        )
        .expect("validate hide_sheet");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply hide_sheet");
    let workbook_xml = entry_xml(&patched.bytes, "xl/workbook.xml");
    assert!(
        workbook_xml.contains(r#"name="Revenue""#)
            && workbook_xml
                .split(r#"name="Revenue""#)
                .nth(1)
                .is_some_and(|tail| {
                    let tag_end = tail.find('>').unwrap_or(tail.len());
                    tail[..tag_end].contains(r#"state="hidden""#)
                }),
        "Revenue sheet must be state=hidden, got: {workbook_xml}"
    );
    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet1.xml");
    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet2.xml");
    assert_untouched_parts(&before, &patched.bytes, &["xl/workbook.xml"]);
    assert_eq!(edit.semantic_diff[0].change, "hide_sheet");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("hidden"));
}

#[test]
fn hide_sheet_rejects_hiding_last_visible_sheet() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let hide_revenue = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "hide_sheet".into(),
                payload: serde_json::json!({ "sheet": "Revenue", "hidden": true }),
            }],
        )
        .expect("validate hide revenue");
    let patched = handler
        .apply_edit(&source, &hide_revenue)
        .expect("hide revenue");
    fs::write(&source, &patched.bytes).expect("rewrite");

    let model = handler.parse(&source).expect("reparse");
    let error = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "hide_sheet".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "hidden": true }),
            }],
        )
        .expect_err("cannot hide last visible sheet");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("last") || message.contains("visible") || message.contains("sole"),
        "unexpected error: {error}"
    );
}

#[test]
fn hide_sheet_unhide_removes_state() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let hide = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "hide_sheet".into(),
                payload: serde_json::json!({ "sheet": "Revenue", "hidden": true }),
            }],
        )
        .expect("validate hide");
    let patched = handler.apply_edit(&source, &hide).expect("hide");
    fs::write(&source, &patched.bytes).expect("rewrite");

    let model = handler.parse(&source).expect("reparse");
    let unhide = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "hide_sheet".into(),
                payload: serde_json::json!({ "sheet": "Revenue", "hidden": false }),
            }],
        )
        .expect("validate unhide");
    let patched = handler.apply_edit(&source, &unhide).expect("unhide");
    let workbook_xml = entry_xml(&patched.bytes, "xl/workbook.xml");
    let revenue_tag = workbook_xml
        .split(r#"name="Revenue""#)
        .nth(1)
        .and_then(|tail| tail.split('>').next())
        .unwrap_or("");
    assert!(
        !revenue_tag.contains("state="),
        "unhide must remove state attr, got: {revenue_tag}"
    );
    assert_eq!(unhide.semantic_diff[0].after.as_deref(), Some("visible"));
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("name");
    inputs.write(0, 0, "Rate").expect("label");
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

fn assert_entry_byte_identical(before: &[u8], after: &[u8], name: &str) {
    assert_eq!(
        entry_bytes(before, name),
        entry_bytes(after, name),
        "{name} must stay byte-identical"
    );
}

fn entry_bytes(package: &[u8], name: &str) -> Vec<u8> {
    let mut archive = ZipArchive::new(Cursor::new(package)).expect("zip");
    let mut entry = archive.by_name(name).expect("entry");
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).expect("read");
    bytes
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
