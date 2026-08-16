use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_docx::{DocxFormat, fixture, parse_document_bytes};
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn set_paragraph_text_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_text".into(),
                payload: serde_json::json!({ "index": 1, "text": "Gamma" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.paragraphs[1].text, "Gamma");
    assert_eq!(after.paragraphs[0].text, "Alpha");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn rejects_tracked_changes() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::tracked_change_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_text".into(),
                payload: serde_json::json!({ "index": 1, "text": "Gamma" }),
            }],
        )
        .expect_err("tracked changes must fail");

    assert!(error.to_string().contains("tracked changes"));
}

#[test]
fn set_paragraph_text_edits_table_cell_and_leaves_header_media_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("table.docx");
    let before = fixture::table_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    assert_eq!(
        model.payload["paragraphs"].as_array().expect("paras").len(),
        3
    );

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_text".into(),
                payload: serde_json::json!({ "index": 1, "text": "UpdatedA" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.paragraphs[0].text, "Intro");
    assert_eq!(after.paragraphs[1].text, "UpdatedA");
    assert_eq!(after.paragraphs[2].text, "CellB");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

fn assert_untouched_entries_identical(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);
    assert_eq!(before_entries.len(), after_entries.len());
    for (name, before_bytes) in &before_entries {
        if patched.contains(&name.as_str()) {
            continue;
        }
        let after_bytes = after_entries.get(name).expect("entry retained");
        assert_eq!(before_bytes, after_bytes, "bytes changed for {name}");
    }
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
