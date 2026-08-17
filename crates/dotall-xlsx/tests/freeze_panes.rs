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
fn inspect_and_parse_expose_freeze_panes_cell() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture_with_freeze(&source, Some((1, 1)));

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let workbook = parse_workbook(&source).expect("parse workbook");
    let inputs = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == "Inputs")
        .expect("Inputs");
    assert_eq!(inputs.freeze_panes.as_deref(), Some("B2"));

    let inspection = handler.inspect(&model).expect("inspect");
    assert_eq!(
        inspection.summary["sheets"][0]["freeze_panes"], "B2",
        "inspect must surface freeze_panes cell"
    );
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "freeze_panes"),
        "capabilities must advertise freeze_panes"
    );
}

#[test]
fn freeze_panes_patches_sheet_view_and_leaves_other_sheet_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture_with_freeze(&source, None);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "freeze_panes".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "cell": "B2" }),
            }],
        )
        .expect("validate freeze_panes");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply freeze_panes");
    let xml = worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains(r#"state="frozen""#)
            && xml.contains(r#"topLeftCell="B2""#)
            && xml.contains(r#"xSplit="1""#)
            && xml.contains(r#"ySplit="1""#),
        "expected frozen pane at B2, got: {}",
        sheet_views_snippet(&xml)
    );
    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet2.xml");
    assert_untouched_non_worksheet_parts(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);

    let after = parse_bytes(&patched.bytes, directory.path());
    let inputs = after
        .sheets
        .iter()
        .find(|sheet| sheet.name == "Inputs")
        .expect("Inputs");
    assert_eq!(inputs.freeze_panes.as_deref(), Some("B2"));
    assert!(inputs.merges.iter().any(|merge| merge == "A1:B1"));
}

#[test]
fn freeze_panes_clear_removes_frozen_pane() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture_with_freeze(&source, Some((1, 1)));

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "freeze_panes".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "cell": null }),
            }],
        )
        .expect("validate clear");
    let patched = handler.apply_edit(&source, &edit).expect("apply clear");
    let xml = worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        !xml.contains(r#"state="frozen""#),
        "frozen pane must be removed: {}",
        sheet_views_snippet(&xml)
    );
    let after = parse_bytes(&patched.bytes, directory.path());
    assert_eq!(after.sheets[0].freeze_panes, None);
}

#[test]
fn freeze_panes_rejects_invalid_cell() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture_with_freeze(&source, None);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let error = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "freeze_panes".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "cell": "1B" }),
            }],
        )
        .expect_err("invalid cell must fail");
    assert!(
        error.to_string().to_lowercase().contains("cell"),
        "expected cell error, got {error}"
    );
}

fn write_fixture_with_freeze(path: &std::path::Path, freeze: Option<(u32, u16)>) {
    let mut workbook = Workbook::new();
    let header = Format::new().set_bold();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("name");
    inputs
        .merge_range(0, 0, 0, 1, "Assumptions", &header)
        .expect("seed merge A1:B1");
    inputs.write(1, 0, "Rate").expect("label");
    inputs.write(1, 1, 0.1).expect("rate");
    if let Some((rows, cols)) = freeze {
        inputs
            .set_freeze_panes(rows, cols)
            .expect("seed freeze panes");
    }
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("name");
    revenue.write(0, 0, "Total").expect("label");
    revenue.write_formula(0, 1, "=Inputs!B2").expect("formula");
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

fn sheet_views_snippet(xml: &str) -> String {
    let start = xml.find("sheetView").unwrap_or(0);
    xml[start..].chars().take(220).collect()
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
