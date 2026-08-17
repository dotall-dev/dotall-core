use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_docx::{DocxFormat, fixture};
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn insert_page_break_prepends_br() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write");
    let handler = DocxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "insert_page_break".into(),
                payload: serde_json::json!({ "index": 1 }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document =
        String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone()).expect("utf8");
    assert!(document.contains(r#"<w:br w:type="page"/>"#), "{document}");
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
