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
fn capabilities_advertise_set_cell_font() {
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
            .any(|cap| cap.operation == "set_cell_font"),
        "capabilities must advertise set_cell_font"
    );
}

#[test]
fn set_cell_font_bold_patches_styles_and_cell_s() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("bytes");
    let handler = XlsxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "set_cell_font".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A1",
                    "bold": true,
                    "name": "Calibri",
                    "size_pt": 14,
                    "color": "#1F4E79"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let styles = entry_xml(&patched.bytes, "xl/styles.xml");
    assert!(styles.contains("<b/>") || styles.contains("<b val=\"1\"/>") || styles.contains("b="));
    assert!(styles.contains("1F4E79") || styles.contains("1f4e79"));
    let sheet = entry_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet.contains("s=\""),
        "cell must reference a cellXf: {sheet}"
    );
    assert_untouched_parts(
        &before,
        &patched.bytes,
        &["xl/worksheets/sheet1.xml", "xl/styles.xml"],
    );
}

#[test]
fn set_cell_font_rejects_empty_payload() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let error = XlsxFormat
        .validate_edit(
            &XlsxFormat.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "set_cell_font".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "address": "A1" }),
            }],
        )
        .expect_err("empty font");
    assert!(error.to_string().contains("font"));
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    inputs.write_string(0, 0, "Hello").expect("write");
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
