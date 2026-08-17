use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_docx::{DocxFormat, fixture, parse_document_bytes};
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn capabilities_advertise_insert_comment() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "insert_comment"),
        "capabilities must advertise insert_comment"
    );
}

#[test]
fn insert_comment_on_paragraph_creates_comments_part_and_markers() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx_with_media(true);
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let paragraph_id = model.payload["paragraphs"][1]["element_id"]
        .as_str()
        .expect("paragraph id")
        .to_owned();

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "index": 1,
                    "text": "Confirm owners",
                    "author": "Dotall"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    fs::write(&path, &patched.bytes).expect("rewrite");
    let reparsed = handler.parse(&path).expect("reparse model");
    let inspection = handler.inspect(&reparsed).expect("inspect");
    let _ = parse_document_bytes(&patched.bytes).expect("reparse bytes");

    let comments = inspection.summary["comments"]
        .as_array()
        .expect("comments array");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["author"], "Dotall");
    assert_eq!(comments[0]["text"], "Confirm owners");
    assert_eq!(comments[0]["index"], 1);
    assert_eq!(comments[0]["paragraph"], paragraph_id);

    let document = entry_text(&patched.bytes, "word/document.xml");
    assert!(
        document.contains("w:commentRangeStart")
            && document.contains("w:commentRangeEnd")
            && document.contains("w:commentReference"),
        "document.xml must contain comment range markers: {document}"
    );
    let comments_xml = entry_text(&patched.bytes, "word/comments.xml");
    assert!(
        comments_xml.contains("Confirm owners") && comments_xml.contains(r#"w:author="Dotall""#),
        "comments.xml must contain inserted comment: {comments_xml}"
    );

    assert_existing_entries_identical_except(
        &before,
        &patched.bytes,
        &[
            "word/document.xml",
            "word/comments.xml",
            "word/_rels/document.xml.rels",
            "[Content_Types].xml",
        ],
    );
    let media_before = zip_entries(&before)
        .remove("word/media/image1.bin")
        .expect("media before");
    let media_after = zip_entries(&patched.bytes)
        .remove("word/media/image1.bin")
        .expect("media after");
    assert_eq!(media_before, media_after);
}

#[test]
fn insert_comment_appends_without_rewriting_existing_comment() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("commented.docx");
    let before = fixture::docx_with_comment_and_media();
    fs::write(&path, &before).expect("write fixture");

    let original_first = FIRST_COMMENT_XML;
    let before_comments = entry_text(&before, "word/comments.xml");
    assert!(
        before_comments.contains(original_first),
        "fixture must contain exact first comment XML"
    );

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "index": 0,
                    "text": "Second note",
                    "author": "Dotall"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    fs::write(&path, &patched.bytes).expect("rewrite");
    let reparsed = handler.parse(&path).expect("reparse model");
    let inspection = handler.inspect(&reparsed).expect("inspect");
    let comments = inspection.summary["comments"]
        .as_array()
        .expect("comments array");
    assert_eq!(comments.len(), 2);

    let first = comments
        .iter()
        .find(|comment| comment["author"] == "Ada")
        .expect("Ada comment retained");
    assert_eq!(first["text"], "Confirm owners");
    assert_eq!(first["index"], 1);

    let second = comments
        .iter()
        .find(|comment| comment["text"] == "Second note")
        .expect("second comment");
    assert_eq!(second["author"], "Dotall");
    assert_eq!(second["index"], 0);

    let comments_xml = entry_text(&patched.bytes, "word/comments.xml");
    let _ = parse_document_bytes(&patched.bytes).expect("reparse bytes");
    assert!(
        comments_xml.contains(original_first),
        "first w:comment inner XML must remain byte-identical substring: {comments_xml}"
    );
    assert!(comments_xml.contains("Second note"));

    assert_existing_entries_identical_except(
        &before,
        &patched.bytes,
        &["word/document.xml", "word/comments.xml"],
    );
}

#[test]
fn set_comment_and_delete_comment_are_rejected() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");

    for kind in ["set_comment", "delete_comment", "replace_comment"] {
        let error = handler
            .validate_edit(
                &model,
                &[SemanticOperation {
                    kind: kind.into(),
                    payload: serde_json::json!({ "index": 1, "text": "Nope" }),
                }],
            )
            .expect_err(kind);
        let message = error.to_string().to_lowercase();
        assert!(
            message.contains("unsupported") || message.contains(kind),
            "{kind} must reject: {error}"
        );
    }
}

const FIRST_COMMENT_XML: &str = r#"<w:comment w:id="0" w:author="Ada" w:date="2024-01-15T12:00:00Z"><w:p><w:r><w:t>Confirm owners</w:t></w:r></w:p></w:comment>"#;

fn entry_text(package: &[u8], name: &str) -> String {
    let bytes = zip_entries(package).remove(name).expect(name);
    String::from_utf8(bytes).expect("utf-8")
}

fn assert_existing_entries_identical_except(before: &[u8], after: &[u8], patched: &[&str]) {
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
