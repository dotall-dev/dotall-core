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
fn capabilities_advertise_set_header_footer() {
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
            .any(|cap| cap.operation == "set_header_footer"),
        "capabilities must advertise set_header_footer"
    );
}

#[test]
fn set_header_footer_patches_worksheet_and_leaves_others_byte_identical() {
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
                kind: "set_header_footer".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "header": "&CBoard pack",
                    "footer": "&P"
                }),
            }],
        )
        .expect("validate set_header_footer");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let sheet_xml = entry_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet_xml.contains("<oddHeader>&amp;CBoard pack</oddHeader>"),
        "expected escaped oddHeader, got: {sheet_xml}"
    );
    assert!(
        sheet_xml.contains("<oddFooter>&amp;P</oddFooter>"),
        "expected escaped oddFooter, got: {sheet_xml}"
    );
    fs::write(&source, &patched.bytes).expect("rewrite");
    let after = parse_workbook(&source).expect("reparse");
    let header_footer = after.sheets[0]
        .header_footer
        .as_ref()
        .expect("header_footer present");
    assert_eq!(header_footer.header.as_deref(), Some("&CBoard pack"));
    assert_eq!(header_footer.footer.as_deref(), Some("&P"));
    assert_eq!(edit.semantic_diff[0].change, "set_header_footer");

    let inspection = handler
        .inspect(&handler.parse(&source).expect("parse after"))
        .expect("inspect");
    assert_eq!(
        inspection.summary["sheets"][0]["header_footer"]["header"],
        "&CBoard pack"
    );
    assert_eq!(
        inspection.summary["sheets"][0]["header_footer"]["footer"],
        "&P"
    );
    assert_untouched_parts(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn set_header_footer_both_null_clears_element() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let set = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_header_footer".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "header": "&CBoard pack",
                    "footer": "&P"
                }),
            }],
        )
        .expect("validate set");
    let patched = handler.apply_edit(&source, &set).expect("apply set");
    fs::write(&source, &patched.bytes).expect("rewrite");

    let model = handler.parse(&source).expect("parse");
    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_header_footer".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "header": null,
                    "footer": null
                }),
            }],
        )
        .expect("validate clear");
    let cleared = handler.apply_edit(&source, &clear).expect("apply clear");
    let sheet_xml = entry_xml(&cleared.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        !sheet_xml.contains("headerFooter") && !sheet_xml.contains("oddHeader"),
        "cleared headerFooter must be removed: {sheet_xml}"
    );
    fs::write(&source, &cleared.bytes).expect("rewrite");
    let after = parse_workbook(&source).expect("reparse");
    assert!(after.sheets[0].header_footer.is_none());

    let inspection = handler
        .inspect(&handler.parse(&source).expect("parse after clear"))
        .expect("inspect");
    assert!(
        inspection.summary["sheets"][0]
            .get("header_footer")
            .is_none_or(|value| value.is_null()),
        "inspect must omit header_footer when absent"
    );
}

#[test]
fn set_header_footer_rejects_missing_sheet() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_header_footer".into(),
                payload: serde_json::json!({
                    "sheet": "Missing",
                    "header": "&CBoard pack"
                }),
            }],
        )
        .expect_err("missing sheet");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("unknown sheet") || message.contains("missing"),
        "unexpected error: {error}"
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
