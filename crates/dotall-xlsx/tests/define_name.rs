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
fn capabilities_advertise_define_name() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source, true);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "define_name"),
        "capabilities must advertise define_name"
    );
}

#[test]
fn define_name_updates_existing_formula_and_leaves_worksheets_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source, true);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "define_name".into(),
                payload: serde_json::json!({
                    "name": "Rate",
                    "formula": "=Inputs!$B$3"
                }),
            }],
        )
        .expect("validate define_name update");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply define_name update");
    let workbook_xml = entry_xml(&patched.bytes, "xl/workbook.xml");
    assert!(
        workbook_xml.contains(r#"name="Rate""#)
            && workbook_xml.contains("Inputs!$B$3")
            && !workbook_xml.contains("Inputs!$B$2"),
        "expected Rate formula updated to Inputs!$B$3, got: {}",
        defined_names_snippet(&workbook_xml)
    );
    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet1.xml");
    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet2.xml");
    assert_untouched_parts(&before, &patched.bytes, &["xl/workbook.xml"]);

    let after = parse_bytes(&patched.bytes, directory.path());
    let rate = after
        .named_ranges
        .iter()
        .find(|range| range.name == "Rate")
        .expect("Rate");
    assert_eq!(rate.formula, "Inputs!$B$3");
    assert!(
        after.sheets[0].merges.iter().any(|merge| merge == "A1:B1"),
        "existing merges must remain"
    );
    assert_eq!(after.sheets[0].freeze_panes.as_deref(), Some("B2"));
}

#[test]
fn define_name_creates_new_name_when_absent() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source, false);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    assert!(
        model.payload["named_ranges"]
            .as_array()
            .map(|ranges| ranges.is_empty())
            .unwrap_or(true)
    );

    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "define_name".into(),
                payload: serde_json::json!({
                    "name": "Base",
                    "formula": "Inputs!$B$3"
                }),
            }],
        )
        .expect("validate create");
    let patched = handler.apply_edit(&source, &edit).expect("apply create");
    let workbook_xml = entry_xml(&patched.bytes, "xl/workbook.xml");
    assert!(
        workbook_xml.contains("<definedNames>")
            && workbook_xml.contains(r#"name="Base""#)
            && workbook_xml.contains("Inputs!$B$3"),
        "expected new definedName Base, got: {}",
        defined_names_snippet(&workbook_xml)
    );

    let after = parse_bytes(&patched.bytes, directory.path());
    let base = after
        .named_ranges
        .iter()
        .find(|range| range.name == "Base")
        .expect("Base");
    assert_eq!(base.formula, "Inputs!$B$3");
}

#[test]
fn define_name_rejects_empty_name_or_formula() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source, true);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let error = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "define_name".into(),
                payload: serde_json::json!({ "name": "  ", "formula": "Inputs!$A$1" }),
            }],
        )
        .expect_err("empty name must fail");
    assert!(
        error.to_string().to_lowercase().contains("name"),
        "expected name error, got {error}"
    );

    let error = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "define_name".into(),
                payload: serde_json::json!({ "name": "Rate", "formula": "=" }),
            }],
        )
        .expect_err("empty formula must fail");
    assert!(
        error.to_string().to_lowercase().contains("formula"),
        "expected formula error, got {error}"
    );
}

fn write_fixture(path: &std::path::Path, with_rate: bool) {
    let mut workbook = Workbook::new();
    let header = Format::new().set_bold();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("name");
    inputs
        .merge_range(0, 0, 0, 1, "Assumptions", &header)
        .expect("seed merge A1:B1");
    inputs.set_freeze_panes(1, 1).expect("seed freeze panes B2");
    inputs.write(1, 0, "Rate").expect("label");
    inputs.write(1, 1, 0.1).expect("rate");
    inputs.write(2, 0, "Base").expect("label");
    inputs.write(2, 1, 100.0).expect("base");
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("name");
    revenue.write(0, 0, "Total").expect("label");
    revenue.write_formula(0, 1, "=Inputs!B2").expect("formula");
    if with_rate {
        workbook
            .define_name("Rate", "=Inputs!$B$2")
            .expect("seed Rate");
    }
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
