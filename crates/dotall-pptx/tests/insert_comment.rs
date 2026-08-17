use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_pptx::{PptxFormat, minimal_pptx, parse_presentation_bytes, pptx_with_table};
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn insert_comment_on_slide1_appears_in_inspect_and_creates_parts() {
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
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "text": "Check KPI",
                    "author": "Dotall",
                    "shape": "Title",
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let out = directory.path().join("after.pptx");
    fs::write(&out, &patched.bytes).expect("write patched");
    let after_model = handler.parse(&out).expect("reparse");
    let inspection = handler.inspect(&after_model).expect("inspect");

    let comments = inspection.summary["comments"]
        .as_array()
        .expect("comments array");
    assert!(
        comments.iter().any(|comment| {
            comment["slide"] == "Slide 1"
                && comment["text"] == "Check KPI"
                && comment["author"] == "Dotall"
                && comment["shape"] == "Title"
                && comment["element_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("cm_"))
        }),
        "expected inserted comment in inspect: {comments:?}"
    );

    let entries = zip_entries(&patched.bytes);
    assert!(
        entries.contains_key("ppt/commentAuthors.xml"),
        "commentAuthors part missing"
    );
    assert!(
        entries
            .keys()
            .any(|name| name.starts_with("ppt/comments/comment") && name.ends_with(".xml")),
        "comments part missing"
    );
    assert_eq!(edit.semantic_diff[0].change, "insert_comment");
}

#[test]
fn insert_comment_leaves_untouched_zip_entries_byte_identical() {
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
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "text": "Check KPI",
                    "author": "Dotall",
                    "shape": "Title",
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.slides[1].shapes[0].text, "Other");

    assert_untouched_entries_identical(
        &before,
        &patched.bytes,
        &[
            "ppt/slides/_rels/slide1.xml.rels",
            "ppt/comments/comment1.xml",
            "ppt/commentAuthors.xml",
            "ppt/_rels/presentation.xml.rels",
            "[Content_Types].xml",
        ],
    );
}

#[test]
fn mutate_existing_comment_kinds_are_rejected() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    for kind in ["set_comment", "delete_comment", "replace_comment"] {
        let error = handler
            .validate_edit(
                &model,
                &[SemanticOperation {
                    kind: kind.into(),
                    payload: serde_json::json!({
                        "slide": "Slide 1",
                        "text": "Nope",
                    }),
                }],
            )
            .expect_err(kind);
        let message = error.to_string().to_lowercase();
        assert!(
            message.contains("unsupported")
                || message.contains("reject")
                || message.contains("not supported")
                || message.contains(kind),
            "unexpected error for {kind}: {error}"
        );
    }
}

#[test]
fn chart_mutate_kind_is_rejected() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_chart_title".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "title": "Hacked",
                }),
            }],
        )
        .expect_err("set_chart_title");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("unsupported")
            || message.contains("reject")
            || message.contains("chart")
            || message.contains("set_chart_title"),
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
