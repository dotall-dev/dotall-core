use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_pptx::{
    PptxFormat, minimal_pptx, parse_presentation_bytes, pptx_with_notes, pptx_with_table,
};
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

#[test]
fn add_slide_after_slide_increases_count_and_preserves_prior_slide_bytes() {
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
                kind: "add_slide".into(),
                payload: serde_json::json!({ "after": "Slide 1" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.slides.len(), 3);
    assert_eq!(after.slides[0].shapes[0].text, "Hello");
    assert_eq!(after.slides[0].tables[0].cells[0].text, "A1");
    assert_eq!(after.slides[2].shapes[0].text, "Other");
    assert!(
        zip_entries(&patched.bytes).contains_key("ppt/slides/slide3.xml"),
        "new slide part should exist"
    );
    assert_eq!(edit.semantic_diff[0].change, "add_slide");
    assert_untouched_entries_identical(
        &before,
        &patched.bytes,
        &[
            "ppt/presentation.xml",
            "ppt/_rels/presentation.xml.rels",
            "[Content_Types].xml",
        ],
    );
}

#[test]
fn delete_slide_removes_last_non_only_slide() {
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
                kind: "delete_slide".into(),
                payload: serde_json::json!({ "slide": "Slide 2" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.slides.len(), 1);
    assert_eq!(after.slides[0].shapes[0].text, "Hello");
    assert!(!zip_entries(&patched.bytes).contains_key("ppt/slides/slide2.xml"));
    assert_eq!(edit.semantic_diff[0].change, "delete_slide");
    assert_untouched_entries_identical(
        &before,
        &patched.bytes,
        &[
            "ppt/presentation.xml",
            "ppt/_rels/presentation.xml.rels",
            "[Content_Types].xml",
            "ppt/slides/slide2.xml",
        ],
    );
}

#[test]
fn set_notes_text_patches_only_notes_part() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    let before = pptx_with_notes();
    fs::write(&path, &before).expect("write fixture");

    let parsed = parse_presentation_bytes(&before).expect("parse bytes");
    assert_eq!(parsed.slides[0].notes.as_deref(), Some("Talk through NPS."));

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_notes_text".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "text": "Updated speaker notes",
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");

    assert_eq!(
        after.slides[0].notes.as_deref(),
        Some("Updated speaker notes")
    );
    assert_eq!(after.slides[0].shapes[0].text, "Hello");
    assert_eq!(edit.semantic_diff[0].change, "set_notes_text");
    assert_eq!(edit.semantic_diff[0].target, "Slide 1!notes");
    assert_untouched_entries_identical(
        &before,
        &patched.bytes,
        &["ppt/notesSlides/notesSlide1.xml"],
    );
}

#[test]
fn set_notes_text_rejects_slide_without_notes() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_notes_text".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "text": "Nope",
                }),
            }],
        )
        .expect_err("no notes");
    let message = error.to_string();
    assert!(
        message.contains("notes") || message.contains("Notes"),
        "unexpected error: {message}"
    );
}

#[test]
fn delete_slide_rejects_sole_slide() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "delete_slide".into(),
                payload: serde_json::json!({ "slide": "Slide 1" }),
            }],
        )
        .expect_err("sole slide");
    let message = error.to_string();
    assert!(
        message.contains("sole") || message.contains("last") || message.contains("only"),
        "unexpected error: {message}"
    );
}

#[test]
fn move_slide_reorders_and_leaves_slide_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    let before = pptx_with_table();
    fs::write(&path, &before).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let before_model = parse_presentation_bytes(&before).expect("parse bytes");
    assert_eq!(before_model.slides.len(), 2);
    assert_eq!(before_model.slides[0].shapes[0].text, "Hello");
    assert_eq!(before_model.slides[1].shapes[0].text, "Other");

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "move_slide".into(),
                payload: serde_json::json!({ "slide": "Slide 2", "to_index": 0 }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.slides.len(), 2);
    assert_eq!(after.slides[0].shapes[0].text, "Other");
    assert_eq!(after.slides[1].shapes[0].text, "Hello");
    assert_eq!(after.slides[1].tables[0].cells[0].text, "A1");
    assert_eq!(edit.semantic_diff[0].change, "move_slide");
    assert_eq!(edit.semantic_diff[0].before.as_deref(), Some("1"));
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("0"));
    // Only presentation.xml changes; slide parts and rels stay byte-identical.
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/presentation.xml"]);
}

#[test]
fn move_slide_keeps_notes_on_title_part_after_reorder() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_notes()).expect("write");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let add = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "add_slide".into(),
                payload: serde_json::json!({ "after": "Slide 1" }),
            }],
        )
        .expect("validate add");
    let with_blank = handler.apply_edit(&path, &add).expect("add");
    fs::write(&path, &with_blank.bytes).expect("rewrite");
    let model = handler.parse(&path).expect("reparse");
    let mov = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "move_slide".into(),
                payload: serde_json::json!({ "slide": "Slide 2", "to_index": 0 }),
            }],
        )
        .expect("validate move");
    let patched = handler.apply_edit(&path, &mov).expect("move");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.slides.len(), 2);
    assert!(after.slides[0].shapes.is_empty());
    assert!(after.slides[0].notes.is_none());
    assert_eq!(after.slides[1].notes.as_deref(), Some("Talk through NPS."));
}

#[test]
fn move_slide_rejects_noop_and_sole_slide() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_table()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let noop = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "move_slide".into(),
                payload: serde_json::json!({ "slide": "Slide 1", "to_index": 0 }),
            }],
        )
        .expect_err("noop");
    assert!(noop.to_string().contains("already"), "unexpected: {noop}");

    let sole_path = directory.path().join("one.pptx");
    fs::write(&sole_path, minimal_pptx()).expect("write sole");
    let sole_model = handler.parse(&sole_path).expect("parse sole");
    let sole = handler
        .validate_edit(
            &sole_model,
            &[SemanticOperation {
                kind: "move_slide".into(),
                payload: serde_json::json!({ "slide": "Slide 1", "to_index": 0 }),
            }],
        )
        .expect_err("sole");
    assert!(
        sole.to_string().contains("fewer than two") || sole.to_string().contains("move_slide"),
        "unexpected: {sole}"
    );
}

#[test]
fn add_textbox_inserts_shape_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    let before = pptx_with_table();
    fs::write(&path, &before).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let before_model = parse_presentation_bytes(&before).expect("parse bytes");
    let before_count = before_model.slides[0].shapes.len();
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "add_textbox".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "name": "Callout",
                    "text": "Agent note",
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.slides[0].shapes.len(), before_count + 1);
    let callout = after.slides[0]
        .shapes
        .iter()
        .find(|shape| shape.name == "Callout")
        .expect("textbox shape");
    assert_eq!(callout.text, "Agent note");
    assert_eq!(after.slides[0].shapes[0].text, "Hello");
    assert_eq!(after.slides[1].shapes[0].text, "Other");
    assert_eq!(edit.semantic_diff[0].change, "add_textbox");
    assert_eq!(edit.semantic_diff[0].target, "Slide 1!Callout");
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/slides/slide1.xml"]);
}

#[test]
fn add_textbox_rejects_missing_slide() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "add_textbox".into(),
                payload: serde_json::json!({
                    "slide": "Slide 99",
                    "text": "Nope",
                }),
            }],
        )
        .expect_err("missing slide");
    let message = error.to_string();
    assert!(
        message.contains("not found") || message.contains("Slide 99"),
        "unexpected error: {message}"
    );
}

#[test]
fn delete_shape_removes_shape_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    let before = pptx_with_table();
    fs::write(&path, &before).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let before_model = parse_presentation_bytes(&before).expect("parse bytes");
    let before_count = before_model.slides[0].shapes.len();
    assert!(before_count >= 1, "fixture needs a shape to delete");

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "delete_shape".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "shape": "Title",
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.slides[0].shapes.len(), before_count - 1);
    assert!(
        after.slides[0]
            .shapes
            .iter()
            .all(|shape| shape.name != "Title"),
        "Title should be removed"
    );
    assert_eq!(
        after.slides[0].tables.len(),
        before_model.slides[0].tables.len()
    );
    assert_eq!(after.slides[1].shapes[0].text, "Other");
    assert_eq!(edit.semantic_diff[0].change, "delete_shape");
    assert_eq!(edit.semantic_diff[0].target, "Slide 1!Title");
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/slides/slide1.xml"]);
}

#[test]
fn delete_shape_rejects_missing_shape() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "delete_shape".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "shape": "MissingBox",
                }),
            }],
        )
        .expect_err("missing shape");
    let message = error.to_string();
    assert!(
        message.contains("not found") || message.contains("MissingBox"),
        "unexpected error: {message}"
    );
}

#[test]
fn rename_shape_updates_name_and_leaves_other_parts_byte_identical() {
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
                kind: "rename_shape".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "shape": "Title",
                    "name": "Headline",
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.slides[0].shapes[0].name, "Headline");
    assert_eq!(after.slides[0].shapes[0].text, "Hello");
    assert_eq!(edit.semantic_diff[0].change, "rename_shape");
    assert_eq!(edit.semantic_diff[0].before.as_deref(), Some("Title"));
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("Headline"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/slides/slide1.xml"]);
}

#[test]
fn rename_shape_rejects_duplicate_name() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_table()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "rename_shape".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "shape": "Title",
                    "name": "Table 1",
                }),
            }],
        )
        .expect_err("duplicate name");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("already") || message.contains("exists") || message.contains("table 1"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_shape_bold_sets_rpr_and_leaves_other_parts_byte_identical() {
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
                kind: "set_shape_bold".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "shape": "Title",
                    "bold": true,
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let slide_xml = String::from_utf8(zip_entries(&patched.bytes)["ppt/slides/slide1.xml"].clone())
        .expect("slide xml");
    assert!(
        slide_xml.contains(r#"b="1""#) || slide_xml.contains("b=\"1\""),
        "expected a:rPr b=1 in slide XML"
    );
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.slides[0].shapes[0].text, "Hello");
    assert_eq!(edit.semantic_diff[0].change, "set_shape_bold");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("true"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/slides/slide1.xml"]);
}

#[test]
fn set_shape_bold_rejects_missing_shape() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_shape_bold".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "shape": "Missing",
                    "bold": true,
                }),
            }],
        )
        .expect_err("missing shape");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("not found") || message.contains("missing"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_shape_italic_sets_rpr_and_leaves_other_parts_byte_identical() {
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
                kind: "set_shape_italic".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "shape": "Title",
                    "italic": true,
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let slide_xml = String::from_utf8(zip_entries(&patched.bytes)["ppt/slides/slide1.xml"].clone())
        .expect("slide xml");
    assert!(
        slide_xml.contains(r#"i="1""#) || slide_xml.contains("i=\"1\""),
        "expected a:rPr i=1 in slide XML"
    );
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.slides[0].shapes[0].text, "Hello");
    assert_eq!(edit.semantic_diff[0].change, "set_shape_italic");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("true"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["ppt/slides/slide1.xml"]);
}

#[test]
fn set_shape_italic_rejects_non_bool() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_shape_italic".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "shape": "Title",
                    "italic": "yes",
                }),
            }],
        )
        .expect_err("non-bool italic");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("italic") || message.contains("boolean") || message.contains("bool"),
        "unexpected error: {error}"
    );
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
