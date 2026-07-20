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
fn patches_a_two_by_three_mixed_range_and_preserves_untouched_zip_entries() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[set_range(
                "Inputs",
                "A1",
                serde_json::json!([
                    [1, {"kind": "formula", "value": "A1*2"}, "three"],
                    [true, null, {"kind": "formula", "value": "=SUM(A1:C1)"}]
                ]),
            )],
        )
        .expect("validate range edit");

    assert_eq!(edit.semantic_diff.len(), 6);
    assert_eq!(edit.semantic_diff[0].target, "Inputs!A1");
    assert_eq!(edit.semantic_diff[5].target, "Inputs!C2");
    assert_eq!(edit.operations.len(), 6);
    assert_eq!(edit.operations[1].payload["formula"], "=A1*2");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply range edit");
    let after = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(cell_value(&after, "Inputs", "A1"), "1");
    assert_eq!(formula(&after, "Inputs", "B1"), "=A1*2");
    assert_eq!(cell_value(&after, "Inputs", "C1"), "three");
    assert_eq!(cell_value(&after, "Inputs", "A2"), "true");
    assert_eq!(cell_value(&after, "Inputs", "B2"), "");
    assert_eq!(formula(&after, "Inputs", "C2"), "=SUM(A1:C1)");
    assert_untouched_entries_are_identical(
        &before,
        &patched.bytes,
        &["xl/worksheets/sheet1.xml", "xl/sharedStrings.xml"],
    );
}

#[test]
fn rejects_ragged_range_values() {
    let handler = XlsxFormat;
    let model = fixture_model(&handler);

    let error = handler
        .validate_edit(
            &model,
            &[set_range("Inputs", "A1", serde_json::json!([[1, 2], [3]]))],
        )
        .expect_err("ragged range must be rejected");

    assert!(
        error.to_string().contains("ragged"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_range_beyond_excel_limits() {
    let handler = XlsxFormat;
    let model = fixture_model(&handler);

    let error = handler
        .validate_edit(
            &model,
            &[set_range(
                "Inputs",
                "XFD1048576",
                serde_json::json!([[1], [2]]),
            )],
        )
        .expect_err("out-of-bounds range must be rejected");

    assert!(
        error.to_string().contains("Excel limits"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_range_exceeding_maximum_cell_count() {
    let handler = XlsxFormat;
    let model = fixture_model(&handler);
    let row = vec![1; 101];
    let values: Vec<Vec<i32>> = vec![row; 101];

    let error = handler
        .validate_edit(
            &model,
            &[set_range("Inputs", "A1", serde_json::json!(values))],
        )
        .expect_err("oversized range must be rejected");

    assert!(
        error.to_string().contains("maximum cell count"),
        "unexpected error: {error}"
    );
}

#[test]
fn leaves_source_unchanged_when_apply_fails_on_last_cell() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_atomicity_fixture(&source);
    let before = range_cell_snapshot(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[set_range(
                "Inputs",
                "A1",
                serde_json::json!([
                    [10, 20, 30],
                    [40, 50, 60]
                ]),
            )],
        )
        .expect("validate range edit");

    replace_zip_entry(&source, "xl/worksheets/sheet1.xml", |xml| {
        xml.replacen(
            r#"<c r="C2"><f>C1+1</f><v>0</v></c>"#,
            r#"<c r="C2"><f t="shared" si="0" ref="C2:C3">C1+1</f><v>0</v></c>"#,
            1,
        )
    });

    let error = handler
        .apply_edit(&source, &edit)
        .expect_err("shared formula cell must fail apply");

    assert!(
        error.to_string().contains("shared"),
        "unexpected error: {error}"
    );
    assert_eq!(range_cell_snapshot(&source), before);
}

#[test]
fn rejects_duplicate_targets_between_range_and_cell_edits() {
    let handler = XlsxFormat;
    let model = fixture_model(&handler);

    let error = handler
        .validate_edit(
            &model,
            &[
                set_range("Inputs", "A1", serde_json::json!([[1, 2]])),
                SemanticOperation {
                    kind: "set_cell_value".into(),
                    payload: serde_json::json!({
                        "sheet": "Inputs",
                        "address": "B1",
                        "value": 3,
                    }),
                },
            ],
        )
        .expect_err("duplicate target must be rejected");

    assert!(
        error
            .to_string()
            .contains("multiple operations target the same cell"),
        "unexpected error: {error}"
    );
}

fn set_range(sheet: &str, start_cell: &str, values: serde_json::Value) -> SemanticOperation {
    SemanticOperation {
        kind: "set_range".into(),
        payload: serde_json::json!({
            "sheet": sheet,
            "start_cell": start_cell,
            "values": values,
        }),
    }
}

fn fixture_model(handler: &XlsxFormat) -> dotall_core::ArtifactEnvelope {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    handler.parse(&source).expect("parse fixture")
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("sheet name");
    inputs.write_string(0, 3, "untouched").expect("input value");
    let summary = workbook
        .add_worksheet()
        .set_name("Summary")
        .expect("sheet name");
    summary
        .write_formula(0, 0, "=Inputs!A1")
        .expect("summary formula");
    workbook.save(path).expect("write fixture");
}

fn write_atomicity_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("sheet name");
    inputs.write_number(0, 0, 1).expect("A1");
    inputs.write_number(0, 1, 2).expect("B1");
    inputs.write_number(0, 2, 3).expect("C1");
    inputs.write_number(1, 0, 4).expect("A2");
    inputs.write_number(1, 1, 5).expect("B2");
    inputs
        .write_formula(1, 2, "=C1+1")
        .expect("C2 formula");
    workbook.save(path).expect("write fixture");
}

fn range_cell_snapshot(path: &std::path::Path) -> BTreeMap<String, String> {
    let workbook = parse_workbook(path).expect("parse fixture");
    ["A1", "B1", "C1", "A2", "B2", "C2"]
        .into_iter()
        .map(|address| {
            (
                address.to_owned(),
                cell_value(&workbook, "Inputs", address),
            )
        })
        .collect()
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

fn parse_workbook_bytes(bytes: &[u8], directory: &std::path::Path) -> dotall_xlsx::WorkbookModel {
    let path = directory.join("patched.xlsx");
    fs::write(&path, bytes).expect("write patched workbook");
    parse_workbook(&path).expect("parse patched workbook")
}

fn cell_value(workbook: &dotall_xlsx::WorkbookModel, sheet: &str, address: &str) -> String {
    workbook
        .sheets
        .iter()
        .find(|candidate| candidate.name == sheet)
        .and_then(|candidate| candidate.cells.iter().find(|cell| cell.address == address))
        .and_then(|cell| dotall_xlsx::edits::format_cell_value(&cell.value))
        .unwrap_or_default()
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
    for (name, before_entry) in &before_entries {
        if patched.contains(&name.as_str()) {
            continue;
        }
        assert_eq!(
            after_entries.get(name),
            Some(before_entry),
            "unchanged ZIP entry differs: {name}"
        );
    }
}

fn zip_entries(bytes: &[u8]) -> BTreeMap<String, ZipEntrySnapshot> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open ZIP");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let name = archive
            .by_index(index)
            .expect("ZIP entry")
            .name()
            .to_owned();
        let (crc32, compressed_size) = {
            let entry = archive.by_index(index).expect("ZIP entry");
            (entry.crc32(), entry.compressed_size())
        };
        let mut raw_compressed = Vec::new();
        archive
            .by_index_raw(index)
            .expect("raw ZIP entry")
            .read_to_end(&mut raw_compressed)
            .expect("read raw ZIP entry");
        let mut inflated = Vec::new();
        archive
            .by_index(index)
            .expect("ZIP entry")
            .read_to_end(&mut inflated)
            .expect("read ZIP entry");
        entries.insert(
            name,
            ZipEntrySnapshot {
                crc32,
                compressed_size,
                inflated,
                raw_compressed,
            },
        );
    }
    entries
}
