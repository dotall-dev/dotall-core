use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::{XlsxFormat, parse_workbook};
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

#[test]
fn patches_a_cell_value_without_changing_other_zip_entries() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A1",
                    "value": 42,
                }),
            }],
        )
        .expect("validate value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply value edit");
    let after_model = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(cell_value(&after_model, "Inputs", "A1"), "42");
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn patches_a_formula_without_changing_other_worksheet() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_formula".into(),
                payload: serde_json::json!({
                    "sheet": "Summary",
                    "address": "B1",
                    "formula": "Inputs!A1*2",
                }),
            }],
        )
        .expect("validate formula edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply formula edit");
    let after_model = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(formula(&after_model, "Summary", "B1"), "=Inputs!A1*2");
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet2.xml"]);
}

#[test]
fn adds_a_sparse_cell_inside_a_valid_worksheet_row() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B2",
                    "value": "created",
                }),
            }],
        )
        .expect("validate sparse value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply sparse value edit");
    let after_model = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(cell_value(&after_model, "Inputs", "B2"), "created");
    assert!(
        worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml")
            .contains(r#"<row r="2"><c r="B2""#),
        "new cells must be nested in their worksheet row"
    );
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn adds_a_cell_to_an_existing_worksheet_row() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B1",
                    "value": "adjacent",
                }),
            }],
        )
        .expect("validate adjacent value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply adjacent value edit");

    assert_eq!(
        cell_value(
            &parse_workbook_bytes(&patched.bytes, directory.path()),
            "Inputs",
            "B1"
        ),
        "adjacent"
    );
    let xml = worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert_eq!(xml.matches(r#"<row r="1""#).count(), 1);
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn resolves_xml_escaped_sheet_names() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    let mut workbook = Workbook::new();
    workbook
        .add_worksheet()
        .set_name("Sales & Ops")
        .expect("sheet name")
        .write_number(0, 0, 1)
        .expect("cell value");
    workbook.save(&source).expect("write fixture");
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Sales & Ops",
                    "address": "A1",
                    "value": 42,
                }),
            }],
        )
        .expect("validate value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply value edit");

    assert_eq!(
        cell_value(
            &parse_workbook_bytes(&patched.bytes, directory.path()),
            "Sales & Ops",
            "A1"
        ),
        "42"
    );
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn adds_a_cell_to_a_row_with_attributes_before_its_reference() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    replace_zip_entry(&source, "xl/worksheets/sheet1.xml", |xml| {
        xml.replacen(r#"<row r="1""#, r#"<row s="1" r="1""#, 1)
    });
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B1",
                    "value": "adjacent",
                }),
            }],
        )
        .expect("validate value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply value edit");
    let xml = worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml");

    assert_eq!(xml.matches(r#"<row "#).count(), 1);
    assert!(xml.contains(r#"<row s="1" r="1""#));
    assert!(xml.contains(r#"<c r="B1" t="inlineStr""#));
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn rejects_edits_to_shared_formula_cells() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Summary",
                    "address": "B1",
                    "value": 42,
                }),
            }],
        )
        .expect("validate value edit");
    replace_zip_entry(&source, "xl/worksheets/sheet2.xml", |xml| {
        xml.replacen(
            "<f>Inputs!A1</f>",
            r#"<f t="shared" si="0" ref="B1:B2">Inputs!A1</f>"#,
            1,
        )
    });

    let error = handler
        .apply_edit(&source, &edit)
        .expect_err("shared formula edits must be rejected");

    assert!(
        error.to_string().contains("shared formula"),
        "unexpected error: {error}"
    );
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("sheet name");
    inputs.write_number(0, 0, 1).expect("input value");
    let summary = workbook
        .add_worksheet()
        .set_name("Summary")
        .expect("sheet name");
    summary
        .write_formula(0, 1, "=Inputs!A1")
        .expect("summary formula");
    workbook.save(path).expect("write fixture");
}

fn parse_workbook_bytes(bytes: &[u8], directory: &std::path::Path) -> dotall_xlsx::WorkbookModel {
    let path = directory.join("patched.xlsx");
    fs::write(&path, bytes).expect("write patched workbook");
    parse_workbook(&path).expect("parse patched workbook")
}

fn cell_value(workbook: &dotall_xlsx::WorkbookModel, sheet: &str, address: &str) -> String {
    let cell = workbook
        .sheets
        .iter()
        .find(|candidate| candidate.name == sheet)
        .and_then(|candidate| candidate.cells.iter().find(|cell| cell.address == address))
        .expect("cell");
    dotall_xlsx::edits::format_cell_value(&cell.value).expect("cell value")
}

fn formula(workbook: &dotall_xlsx::WorkbookModel, sheet: &str, address: &str) -> String {
    workbook
        .sheets
        .iter()
        .find(|candidate| candidate.name == sheet)
        .and_then(|candidate| candidate.cells.iter().find(|cell| cell.address == address))
        .and_then(|cell| cell.formula.clone())
        .expect("formula")
}

#[derive(Debug, PartialEq, Eq)]
struct ZipEntrySnapshot {
    crc32: u32,
    compression: zip::CompressionMethod,
    compressed_size: u64,
    inflated: Vec<u8>,
    raw_compressed: Vec<u8>,
}

fn assert_untouched_entries_are_identical(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);

    assert_eq!(
        before_entries.len(),
        after_entries.len(),
        "ZIP entry count changed"
    );
    assert!(
        before_entries.contains_key("xl/styles.xml"),
        "fixture must include styles.xml"
    );
    for (name, before_entry) in &before_entries {
        let after_entry = after_entries.get(name).expect("entry retained");
        if patched.contains(&name.as_str()) {
            continue;
        }
        assert_eq!(
            before_entry.crc32, after_entry.crc32,
            "CRC changed for {name}"
        );
        assert_eq!(
            before_entry.compression, after_entry.compression,
            "compression method changed for {name}"
        );
        assert_eq!(
            before_entry.compressed_size, after_entry.compressed_size,
            "compressed size changed for {name}"
        );
        assert_eq!(
            before_entry.inflated, after_entry.inflated,
            "inflated bytes changed for {name}"
        );
        assert_eq!(
            before_entry.raw_compressed, after_entry.raw_compressed,
            "raw compressed payload changed for {name}"
        );
    }
}

fn zip_entries(bytes: &[u8]) -> BTreeMap<String, ZipEntrySnapshot> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open ZIP");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let name = {
            let entry = archive.by_index(index).expect("ZIP entry");
            entry.name().to_owned()
        };
        let (crc32, compression, compressed_size) = {
            let entry = archive.by_index(index).expect("ZIP entry");
            (
                entry.crc32(),
                entry.compression(),
                entry.compressed_size(),
            )
        };
        let mut raw_compressed = Vec::new();
        archive
            .by_index_raw(index)
            .expect("raw ZIP entry")
            .read_to_end(&mut raw_compressed)
            .expect("read raw compressed payload");
        let mut inflated = Vec::new();
        archive
            .by_index(index)
            .expect("ZIP entry")
            .read_to_end(&mut inflated)
            .expect("read inflated ZIP entry");
        entries.insert(
            name,
            ZipEntrySnapshot {
                crc32,
                compression,
                compressed_size,
                inflated,
                raw_compressed,
            },
        );
    }
    entries
}

fn worksheet_xml(bytes: &[u8], name: &str) -> String {
    let entries = zip_entries(bytes);
    String::from_utf8(entries[name].inflated.clone()).expect("worksheet XML")
}

fn replace_zip_entry(path: &std::path::Path, name: &str, replace: impl FnOnce(String) -> String) {
    let source = fs::read(path).expect("fixture bytes");
    let mut archive = ZipArchive::new(Cursor::new(source)).expect("open ZIP");
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    let mut replace = Some(replace);

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("ZIP entry");
        let entry_name = entry.name().to_owned();
        if entry_name == name {
            let mut xml = String::new();
            entry.read_to_string(&mut xml).expect("read worksheet XML");
            let replacement = replace.take().expect("replace exactly once")(xml);
            output
                .start_file(
                    entry_name,
                    SimpleFileOptions::default().compression_method(entry.compression()),
                )
                .expect("start replacement entry");
            output
                .write_all(replacement.as_bytes())
                .expect("write replacement entry");
        } else {
            output.raw_copy_file(entry).expect("copy ZIP entry");
        }
    }
    fs::write(path, output.finish().expect("finish ZIP").into_inner()).expect("write fixture");
}
