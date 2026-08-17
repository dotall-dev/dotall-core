use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use base64::Engine;
use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_docx::{DocxFormat, fixture, parse_document_bytes};
use tempfile::tempdir;
use zip::ZipArchive;

const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[test]
fn capabilities_advertise_insert_picture() {
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
            .any(|cap| cap.operation == "insert_picture"),
        "capabilities must advertise insert_picture"
    );
}

#[test]
fn inspect_lists_pictures() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("picture.docx");
    fs::write(&path, fixture::docx_with_picture()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    let pictures = inspection.summary["pictures"]
        .as_array()
        .expect("pictures array");
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0]["index"], 0);
    let part = pictures[0]["part"].as_str().expect("part");
    assert!(
        part.ends_with("image1.png") || part == "word/media/image1.png",
        "expected image1.png part, got {part}"
    );
    let element_id = pictures[0]["element_id"]
        .as_str()
        .expect("picture element_id");
    assert!(
        element_id.starts_with("pic_"),
        "expected opaque pic_ id, got {element_id}"
    );
    assert!(
        inspection.summary["comments"]
            .as_array()
            .is_some_and(|items| items.is_empty()),
        "comments must remain an array"
    );
    assert!(
        inspection.summary["charts"]
            .as_array()
            .is_some_and(|items| items.is_empty()),
        "charts must remain an array"
    );
}

#[test]
fn inspect_emits_empty_pictures_when_absent() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    let pictures = inspection.summary["pictures"]
        .as_array()
        .expect("pictures always emitted");
    assert!(pictures.is_empty());
}

#[test]
fn insert_picture_creates_media_on_minimal_docx() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);
    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_picture".into(),
                payload: serde_json::json!({
                    "index": 1,
                    "bytes_base64": encoded,
                    "content_type": "image/png"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    fs::write(&path, &patched.bytes).expect("rewrite");
    let reparsed = handler.parse(&path).expect("reparse");
    let inspection = handler.inspect(&reparsed).expect("inspect");
    let _ = parse_document_bytes(&patched.bytes).expect("reparse bytes");

    let pictures = inspection.summary["pictures"]
        .as_array()
        .expect("pictures array");
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0]["index"], 1);
    let part = pictures[0]["part"].as_str().expect("part");
    assert!(
        part.contains("image1.png"),
        "expected word/media/image1.png, got {part}"
    );

    let media = zip_entries(&patched.bytes)
        .remove("word/media/image1.png")
        .expect("media part");
    assert_eq!(media, TINY_PNG);

    let document = entry_text(&patched.bytes, "word/document.xml");
    assert!(
        document.contains("w:drawing")
            && document.contains("wp:inline")
            && document.contains("a:blip"),
        "document.xml must contain inline drawing: {document}"
    );
    let rels = entry_text(&patched.bytes, "word/_rels/document.xml.rels");
    assert!(
        rels.contains("relationships/image") && rels.contains("media/image1.png"),
        "document rels must link image: {rels}"
    );
    let content_types = entry_text(&patched.bytes, "[Content_Types].xml");
    assert!(
        content_types.contains(r#"Extension="png""#) && content_types.contains("image/png"),
        "Content_Types must declare png: {content_types}"
    );
}

#[test]
fn insert_picture_does_not_rewrite_existing_media_bytes() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx_with_media(true);
    fs::write(&path, &before).expect("write fixture");

    let media_before = zip_entries(&before)
        .remove("word/media/image1.bin")
        .expect("media before");

    let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);
    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_picture".into(),
                payload: serde_json::json!({
                    "index": 1,
                    "bytes_base64": encoded
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    fs::write(&path, &patched.bytes).expect("rewrite");
    let reparsed = handler.parse(&path).expect("reparse");
    let inspection = handler.inspect(&reparsed).expect("inspect");

    let media_after = zip_entries(&patched.bytes)
        .remove("word/media/image1.bin")
        .expect("media after");
    assert_eq!(media_before, media_after);

    let pictures = inspection.summary["pictures"]
        .as_array()
        .expect("pictures array");
    assert_eq!(pictures.len(), 1);
    let part = pictures[0]["part"].as_str().expect("part");
    assert!(
        part.contains("image2.png"),
        "next media part should be image2.png when image1.bin exists, got {part}"
    );

    assert_existing_entries_identical_except(
        &before,
        &patched.bytes,
        &[
            "word/document.xml",
            "word/_rels/document.xml.rels",
            "[Content_Types].xml",
            "word/media/image2.png",
        ],
    );
}

#[test]
fn replace_picture_delete_picture_set_picture_are_rejected() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");

    for kind in ["replace_picture", "delete_picture", "set_picture"] {
        let error = handler
            .validate_edit(
                &model,
                &[SemanticOperation {
                    kind: kind.into(),
                    payload: serde_json::json!({
                        "index": 1,
                        "bytes_base64": "AAAA"
                    }),
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
