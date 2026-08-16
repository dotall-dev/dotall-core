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
fn merge_cells_adds_merge_and_leaves_other_sheet_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_two_sheet_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "merge_cells".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "range": "A2:B3" }),
            }],
        )
        .expect("validate merge_cells");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply merge_cells");
    let after = parse_bytes(&patched.bytes, directory.path());
    let inputs = after
        .sheets
        .iter()
        .find(|sheet| sheet.name == "Inputs")
        .expect("Inputs");
    assert!(
        inputs.merges.iter().any(|merge| merge == "A2:B3"),
        "expected A2:B3 in {:?}",
        inputs.merges
    );
    assert!(
        worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml")
            .contains(r#"<mergeCell ref="A2:B3"/>"#)
    );
    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet2.xml");
    assert_untouched_non_worksheet_parts(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn merge_cells_rejects_overlapping_merges() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_two_sheet_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let error = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "merge_cells".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "range": "A1:A2" }),
            }],
        )
        .expect_err("overlapping merge must fail");

    let message = error.to_string();
    assert!(
        message.to_lowercase().contains("overlap"),
        "expected overlap error, got {message}"
    );
}

#[test]
fn unmerge_cells_removes_matching_ref() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_two_sheet_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "unmerge_cells".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "range": "A1:B1" }),
            }],
        )
        .expect("validate unmerge_cells");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply unmerge_cells");
    let after = parse_bytes(&patched.bytes, directory.path());
    let inputs = after
        .sheets
        .iter()
        .find(|sheet| sheet.name == "Inputs")
        .expect("Inputs");
    assert!(
        !inputs.merges.iter().any(|merge| merge == "A1:B1"),
        "merge should be removed, got {:?}",
        inputs.merges
    );
    assert!(!worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml").contains("mergeCell"));
    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet2.xml");
}

fn write_two_sheet_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let header = Format::new().set_bold();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("name");
    inputs
        .merge_range(0, 0, 0, 1, "Assumptions", &header)
        .expect("seed merge A1:B1");
    inputs.write(1, 0, "Rate").expect("label");
    inputs.write(1, 1, 0.1).expect("rate");
    inputs.write(2, 0, "Base").expect("base label");
    inputs.write(2, 1, 100).expect("base");
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("name");
    revenue.write(0, 0, "Total").expect("label");
    revenue.write_formula(0, 1, "=Inputs!B3").expect("formula");
    workbook.save(path).expect("save fixture");
}

fn parse_bytes(bytes: &[u8], directory: &std::path::Path) -> dotall_xlsx::WorkbookModel {
    let path = directory.join("patched.xlsx");
    fs::write(&path, bytes).expect("write patched workbook");
    parse_workbook(&path).expect("parse patched workbook")
}

fn worksheet_xml(bytes: &[u8], name: &str) -> String {
    String::from_utf8(zip_entries(bytes)[name].clone()).expect("worksheet XML")
}

fn assert_entry_byte_identical(before: &[u8], after: &[u8], name: &str) {
    assert_eq!(
        zip_entries(before).get(name),
        zip_entries(after).get(name),
        "entry `{name}` must stay byte-identical"
    );
}

fn assert_untouched_non_worksheet_parts(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);
    for (name, before_bytes) in &before_entries {
        if patched.contains(&name.as_str()) || name.starts_with("xl/worksheets/") {
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
