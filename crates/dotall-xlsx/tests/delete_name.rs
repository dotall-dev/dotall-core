use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::{XlsxFormat, parse_workbook};
use rust_xlsxwriter::{Format, Workbook};
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn capabilities_advertise_delete_name() {
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
            .any(|cap| cap.operation == "delete_name"),
        "capabilities must advertise delete_name"
    );
}

#[test]
fn delete_name_removes_defined_name_and_leaves_worksheets_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    assert!(
        model.payload["named_ranges"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|range| range["name"] == "Rate")
    );

    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "delete_name".into(),
                payload: serde_json::json!({ "name": "Rate" }),
            }],
        )
        .expect("validate delete_name");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply delete_name");
    let workbook_xml = entry_xml(&patched.bytes, "xl/workbook.xml");
    assert!(
        !workbook_xml.contains(r#"name="Rate""#),
        "Rate definedName must be removed, got: {}",
        defined_names_snippet(&workbook_xml)
    );
    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet1.xml");
    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet2.xml");
    assert_untouched_parts(&before, &patched.bytes, &["xl/workbook.xml"]);

    let after = parse_bytes(&patched.bytes, directory.path());
    assert!(
        after
            .named_ranges
            .iter()
            .all(|range| !range.name.eq_ignore_ascii_case("Rate")),
        "Rate must be gone from model"
    );
    assert_eq!(edit.semantic_diff[0].change, "delete_name");
}

#[test]
fn delete_name_rejects_missing_name() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let error = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "delete_name".into(),
                payload: serde_json::json!({ "name": "MissingName" }),
            }],
        )
        .expect_err("missing name must fail");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("not found") || message.contains("missing"),
        "unexpected error: {error}"
    );
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let header = Format::new().set_bold();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("name");
    inputs
        .merge_range(0, 0, 0, 1, "Assumptions", &header)
        .expect("seed merge");
    inputs.write(1, 0, "Rate").expect("label");
    inputs.write(1, 1, 0.1).expect("rate");
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("name");
    revenue.write(0, 0, "Total").expect("label");
    revenue.write_formula(0, 1, "=Inputs!B2").expect("formula");
    workbook
        .define_name("Rate", "=Inputs!$B$2")
        .expect("seed Rate");
    workbook.save(path).expect("save fixture");
}

fn parse_bytes(bytes: &[u8], directory: &std::path::Path) -> dotall_xlsx::WorkbookModel {
    let path = directory.join("patched.xlsx");
    fs::write(&path, bytes).expect("write patched workbook");
    parse_workbook(&path).expect("parse patched workbook")
}

fn entry_xml(bytes: &[u8], name: &str) -> String {
    String::from_utf8(zip_entries(bytes)[name].clone()).expect("entry XML")
}

fn defined_names_snippet(xml: &str) -> String {
    let start = xml.find("definedName").unwrap_or(0);
    xml[start..].chars().take(220).collect()
}

fn assert_entry_byte_identical(before: &[u8], after: &[u8], name: &str) {
    assert_eq!(
        zip_entries(before).get(name),
        zip_entries(after).get(name),
        "entry `{name}` must stay byte-identical"
    );
}

fn assert_untouched_parts(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);
    for (name, before_bytes) in &before_entries {
        if patched.contains(&name.as_str()) {
            continue;
        }
        let after_bytes = after_entries.get(name).expect("entry retained");
        assert_eq!(before_bytes, after_bytes, "untouched part `{name}` changed");
    }
}

fn zip_entries(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open ZIP");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("ZIP entry");
        let name = entry.name().to_owned();
        let mut inflated = Vec::new();
        entry.read_to_end(&mut inflated).expect("read entry");
        entries.insert(name, inflated);
    }
    entries
}
