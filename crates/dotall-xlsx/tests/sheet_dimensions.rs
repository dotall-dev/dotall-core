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
fn set_column_width_patches_cols_and_leaves_other_sheet_byte_identical() {
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
                kind: "set_column_width".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "column": "A", "width": 22.5 }),
            }],
        )
        .expect("validate set_column_width");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply set_column_width");
    let xml = worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains(r#"min="1""#) && xml.contains(r#"max="1""#),
        "expected single-column col entry, got snippet around cols: {}",
        cols_snippet(&xml)
    );
    assert!(
        xml.contains(r#"width="22.5""#) && xml.contains(r#"customWidth="1""#),
        "expected width=22.5 customWidth=1 in {}",
        cols_snippet(&xml)
    );
    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet2.xml");
    assert_untouched_non_worksheet_parts(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);

    // Prior paths remain healthy after a dimension write.
    let after = parse_bytes(&patched.bytes, directory.path());
    let inputs = after
        .sheets
        .iter()
        .find(|sheet| sheet.name == "Inputs")
        .expect("Inputs");
    assert!(inputs.merges.iter().any(|merge| merge == "A1:B1"));
}

#[test]
fn set_row_height_patches_row_and_leaves_other_sheet_byte_identical() {
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
                kind: "set_row_height".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "row": 1, "height": 36.0 }),
            }],
        )
        .expect("validate set_row_height");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply set_row_height");
    let xml = worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains(r#"<row r="1""#)
            && xml.contains(r#"ht="36""#)
            && xml.contains(r#"customHeight="1""#),
        "expected row 1 height 36, got row tag: {}",
        row_tag_snippet(&xml, 1)
    );
    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet2.xml");
    assert_untouched_non_worksheet_parts(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn set_column_width_rejects_invalid_column_and_non_positive_width() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_two_sheet_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");

    let bad_column = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "set_column_width".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "column": "1A", "width": 10 }),
            }],
        )
        .expect_err("invalid column must fail");
    assert!(
        bad_column.to_string().to_lowercase().contains("column"),
        "expected column error, got {bad_column}"
    );

    let bad_width = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "set_column_width".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "column": "B", "width": 0 }),
            }],
        )
        .expect_err("non-positive width must fail");
    assert!(
        bad_width.to_string().to_lowercase().contains("width"),
        "expected width error, got {bad_width}"
    );
}

#[test]
fn set_row_height_rejects_out_of_range_row() {
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
                kind: "set_row_height".into(),
                payload: serde_json::json!({ "sheet": "Inputs", "row": 0, "height": 20 }),
            }],
        )
        .expect_err("row 0 must fail");
    assert!(
        error.to_string().to_lowercase().contains("row"),
        "expected row error, got {error}"
    );
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

fn cols_snippet(xml: &str) -> String {
    let start = xml.find("<cols").unwrap_or(0);
    xml[start..].chars().take(160).collect()
}

fn row_tag_snippet(xml: &str, row: u32) -> String {
    let needle = format!(r#"<row r="{row}""#);
    let start = xml.find(&needle).unwrap_or(0);
    xml[start..].chars().take(120).collect()
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
