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
fn capabilities_advertise_set_paper_size() {
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
            .any(|cap| cap.operation == "set_paper_size"),
        "capabilities must advertise set_paper_size"
    );
}

#[test]
fn set_paper_size_patches_worksheet_and_leaves_others_byte_identical() {
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
                kind: "set_paper_size".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "paper_size": 9
                }),
            }],
        )
        .expect("validate set_paper_size");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let sheet_xml = entry_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet_xml.contains(r#"paperSize="9""#),
        "expected pageSetup paperSize=9, got: {sheet_xml}"
    );
    fs::write(&source, &patched.bytes).expect("rewrite");
    let after = parse_workbook(&source).expect("reparse");
    assert_eq!(after.sheets[0].paper_size, Some(9));
    assert_eq!(edit.semantic_diff[0].change, "set_paper_size");

    let inspection = handler
        .inspect(&handler.parse(&source).expect("parse after"))
        .expect("inspect");
    assert_eq!(inspection.summary["sheets"][0]["paper_size"], 9);
    assert_untouched_parts(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn set_paper_size_rejects_zero() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paper_size".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "paper_size": 0 }),
            }],
        )
        .expect_err("zero paper_size");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("paper_size") || message.contains("positive"),
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
