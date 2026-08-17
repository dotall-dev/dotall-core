use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_pptx::{PptxFormat, parse_presentation_bytes, pptx_with_table};
use tempfile::tempdir;
use zip::ZipArchive;

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

fn assert_untouched_entries_identical(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);
    for (name, before_bytes) in &before_entries {
        if patched.contains(&name.as_str()) {
            continue;
        }
        let after_bytes = after_entries
            .get(name)
            .unwrap_or_else(|| panic!("entry retained: {name}"));
        assert_eq!(before_bytes, after_bytes, "bytes changed for {name}");
    }
}

#[test]
fn capabilities_advertise_set_table_cell_italic() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_table()).expect("write fixture");
    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "set_table_cell_italic"),
        "capabilities must advertise set_table_cell_italic"
    );
}

#[test]
fn set_table_cell_italic_patches_only_target_slide() {
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
                kind: "set_table_cell_italic".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "table": "Table 1",
                    "row": 0,
                    "col": 1,
                    "italic": true,
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let slide_xml = String::from_utf8(zip_entries(&patched.bytes)["ppt/slides/slide1.xml"].clone())
        .expect("slide xml");
    assert!(
        slide_xml.contains(r#"<a:rPr i="1"/><a:t>B1</a:t>"#) || slide_xml.contains(r#"i="1""#),
        "expected italic on B1: {slide_xml}"
    );
    assert!(
        !slide_xml.contains(r#"<a:rPr i="1"/><a:t>A1</a:t>"#),
        "sibling cell A1 must not be italicized"
    );
    assert!(
        !slide_xml.contains(r#"<a:rPr i="1"/><a:t>A2</a:t>"#)
            && !slide_xml.contains(r#"<a:rPr i="1"/><a:t>B2</a:t>"#),
        "other table cells must not be italicized"
    );
    assert!(
        !slide_xml.contains(r#"<a:rPr i="1"/><a:t>Hello</a:t>"#),
        "title shape text must not be italicized"
    );

    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");
    let cells = &after.slides[0].tables[0].cells;
    assert_eq!(
        cells
            .iter()
            .find(|cell| cell.row == 0 && cell.col == 0)
            .map(|cell| cell.text.as_str()),
        Some("A1")
    );
    assert_eq!(
        cells
            .iter()
            .find(|cell| cell.row == 0 && cell.col == 1)
            .map(|cell| cell.text.as_str()),
        Some("B1")
    );
    assert_eq!(after.slides[0].shapes[0].text, "Hello");
    assert_eq!(after.slides[1].shapes[0].text, "Other");
    assert_eq!(edit.semantic_diff[0].change, "set_table_cell_italic");
    assert_eq!(
        edit.semantic_diff[0].target, "Slide 1!Table 1!r0c1",
        "validated edit should name the cell"
    );
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("true"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/slides/slide1.xml"]);
}

#[test]
fn set_table_cell_italic_false_writes_i_zero() {
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
                kind: "set_table_cell_italic".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "table": "Table 1",
                    "row": 0,
                    "col": 1,
                    "italic": false,
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let slide_xml = String::from_utf8(zip_entries(&patched.bytes)["ppt/slides/slide1.xml"].clone())
        .expect("slide xml");
    assert!(
        slide_xml.contains(r#"<a:rPr i="0"/><a:t>B1</a:t>"#),
        "expected a:rPr i=0 on target cell B1 (same 1/0 convention as set_shape_italic), got: {slide_xml}"
    );
    assert!(
        !slide_xml.contains(r#"<a:rPr i="0"/><a:t>A1</a:t>"#),
        "sibling cell A1 must not be altered"
    );
    assert!(
        !slide_xml.contains(r#"<a:rPr i="0"/><a:t>Hello</a:t>"#),
        "title shape text must not be altered"
    );
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.slides[0].shapes[0].text, "Hello");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("false"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/slides/slide1.xml"]);
}

#[test]
fn set_table_cell_italic_rejects_out_of_range() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_table()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_table_cell_italic".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "table": "Table 1",
                    "row": 9,
                    "col": 0,
                    "italic": true,
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

#[test]
fn set_table_cell_italic_rejects_unknown_table() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_table()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_table_cell_italic".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "table": "Missing Table",
                    "row": 0,
                    "col": 0,
                    "italic": true,
                }),
            }],
        )
        .expect_err("unknown table");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("not found") || message.contains("missing"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_table_cell_italic_rejects_unknown_slide() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_table()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_table_cell_italic".into(),
                payload: serde_json::json!({
                    "slide": "Slide 99",
                    "table": "Table 1",
                    "row": 0,
                    "col": 0,
                    "italic": true,
                }),
            }],
        )
        .expect_err("unknown slide");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("not found") || message.contains("missing"),
        "unexpected error: {error}"
    );
}
