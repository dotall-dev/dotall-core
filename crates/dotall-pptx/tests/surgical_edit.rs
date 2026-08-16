use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_pptx::{PptxFormat, minimal_pptx, parse_presentation_bytes, pptx_with_table};
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn set_shape_text_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    let before = minimal_pptx();
    fs::write(&path, &before).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_shape_text".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "shape": "Title",
                    "text": "World",
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.slides[0].shapes[0].text, "World");
    assert_eq!(
        edit.semantic_diff[0].target, "Slide 1!Title",
        "validated edit should name the shape"
    );
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/slides/slide1.xml"]);
}

#[test]
fn set_table_cell_text_patches_only_target_slide() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    let before = pptx_with_table();
    fs::write(&path, &before).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_table_cell_text".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "table": "Table 1",
                    "row": 0,
                    "col": 1,
                    "text": "NEW",
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");

    let cells = &after.slides[0].tables[0].cells;
    assert_eq!(
        cells
            .iter()
            .find(|cell| cell.row == 0 && cell.col == 1)
            .map(|cell| cell.text.as_str()),
        Some("NEW")
    );
    assert_eq!(
        cells
            .iter()
            .find(|cell| cell.row == 0 && cell.col == 0)
            .map(|cell| cell.text.as_str()),
        Some("A1")
    );
    assert_eq!(after.slides[0].shapes[0].text, "Hello");
    assert_eq!(after.slides[1].shapes[0].text, "Other");
    assert_eq!(
        edit.semantic_diff[0].target, "Slide 1!Table 1!r0c1",
        "validated edit should name the cell"
    );
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/slides/slide1.xml"]);
}

#[test]
fn set_table_cell_text_rejects_out_of_range() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_table()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_table_cell_text".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "table": "Table 1",
                    "row": 9,
                    "col": 0,
                    "text": "Nope",
                }),
            }],
        )
        .expect_err("out of range");
    let message = error.to_string();
    assert!(
        message.contains("out of range") || message.contains("row"),
        "unexpected error: {message}"
    );
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
