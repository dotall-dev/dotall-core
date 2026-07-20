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
fn reject_policy_lists_formula_and_defined_name_references() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_referenced_fixture(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let error = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[delete_sheet("Inputs", "reject_if_referenced")],
        )
        .expect_err("referenced sheet must be rejected");

    let message = error.to_string();
    assert!(
        message.contains("Summary!A1"),
        "unexpected error: {message}"
    );
    assert!(message.contains("TaxRate"), "unexpected error: {message}");
}

#[test]
fn replace_policy_rewrites_references_and_removes_sheet_parts() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_referenced_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[delete_sheet("Inputs", "replace_references_with_ref_error")],
        )
        .expect("replace policy validates");
    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply sheet delete");
    let after = parse_workbook_bytes(&patched.bytes, directory.path());
    let entries = zip_entries(&patched.bytes);

    assert_eq!(after.sheets.len(), 1);
    assert_eq!(after.sheets[0].name, "Summary");
    assert_eq!(after.sheets[0].cells[0].formula.as_deref(), Some("=#REF!"));
    assert_eq!(
        after.named_ranges[0].formula, "#REF!",
        "defined name is rewritten"
    );
    assert!(!entries.contains_key("xl/worksheets/sheet1.xml"));
    assert!(!entries.contains_key("xl/worksheets/_rels/sheet1.xml.rels"));
    assert!(!worksheet_xml(&patched.bytes, "xl/workbook.xml").contains("Inputs"));
    assert!(!worksheet_xml(&patched.bytes, "xl/_rels/workbook.xml.rels").contains("sheet1.xml"));
    assert!(!worksheet_xml(&patched.bytes, "[Content_Types].xml").contains("sheet1.xml"));
    assert_untouched_entries_are_identical(
        &before,
        &patched.bytes,
        &[
            "xl/workbook.xml",
            "xl/_rels/workbook.xml.rels",
            "[Content_Types].xml",
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "docProps/app.xml",
        ],
    );
}

#[test]
fn rejects_deleting_the_last_visible_sheet() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    let mut workbook = Workbook::new();
    workbook
        .add_worksheet()
        .set_name("Only")
        .expect("sheet name");
    workbook.save(&source).expect("write fixture");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let error = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[delete_sheet("Only", "reject_if_referenced")],
        )
        .expect_err("last visible sheet must be rejected");

    assert!(
        error.to_string().contains("last visible sheet"),
        "unexpected error: {error}"
    );
}

fn delete_sheet(name: &str, dependency_policy: &str) -> SemanticOperation {
    SemanticOperation {
        kind: "delete_sheet".into(),
        payload: serde_json::json!({
            "name": name,
            "dependency_policy": dependency_policy,
        }),
    }
}

fn write_referenced_fixture(path: &std::path::Path) {
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
        .write_formula(0, 0, "=Inputs!A1")
        .expect("summary formula");
    workbook.save(path).expect("write fixture");

    replace_zip_entry(path, "xl/workbook.xml", |xml| {
        xml.replace(
            "</workbook>",
            "<definedNames><definedName name=\"TaxRate\">Inputs!$A$1</definedName></definedNames></workbook>",
        )
    });
}

fn parse_workbook_bytes(bytes: &[u8], directory: &std::path::Path) -> dotall_xlsx::WorkbookModel {
    let path = directory.join("patched.xlsx");
    fs::write(&path, bytes).expect("write patched workbook");
    parse_workbook(&path).expect("parse patched workbook")
}

#[derive(Debug, PartialEq, Eq)]
struct ZipEntrySnapshot {
    crc32: u32,
    compressed_size: u64,
    raw_compressed: Vec<u8>,
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
        entries.insert(
            name,
            ZipEntrySnapshot {
                crc32,
                compressed_size,
                raw_compressed,
            },
        );
    }
    entries
}

fn assert_untouched_entries_are_identical(before: &[u8], after: &[u8], modified: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);
    for (name, before_entry) in &before_entries {
        if modified.contains(&name.as_str()) {
            continue;
        }
        assert_eq!(
            after_entries.get(name),
            Some(before_entry),
            "unchanged ZIP entry differs: {name}"
        );
    }
}

fn worksheet_xml(bytes: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open ZIP");
    let mut xml = String::new();
    archive
        .by_name(name)
        .expect("ZIP entry")
        .read_to_string(&mut xml)
        .expect("read XML");
    xml
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
            entry.read_to_string(&mut xml).expect("read XML");
            output
                .start_file(entry_name, SimpleFileOptions::default())
                .expect("start replacement");
            output
                .write_all(replace.take().expect("replace once")(xml).as_bytes())
                .expect("write replacement");
        } else {
            output.raw_copy_file(entry).expect("copy ZIP entry");
        }
    }
    fs::write(path, output.finish().expect("finish ZIP").into_inner()).expect("write fixture");
}
