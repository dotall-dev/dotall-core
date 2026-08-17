use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use base64::Engine;
use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_pptx::{
    PptxFormat, minimal_pptx, minimal_pptx_with_media, parse_presentation_bytes, pptx_with_png,
};
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
fn inspect_lists_pictures_from_png_fixture() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("logo.pptx");
    fs::write(&path, pptx_with_png()).expect("write fixture");

    let handler = PptxFormat;
    let inspection = handler
        .inspect(&handler.parse(&path).expect("parse"))
        .expect("inspect");
    let pictures = inspection.summary["pictures"]
        .as_array()
        .expect("pictures array");
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0]["slide"], "Slide 1");
    assert_eq!(pictures[0]["name"], "Logo");
    assert!(
        pictures[0]["part"]
            .as_str()
            .expect("part")
            .ends_with("image1.png"),
        "expected image1.png part: {:?}",
        pictures[0]["part"]
    );
    assert!(
        pictures[0]["element_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("pic_")),
        "expected pic_ element_id"
    );
    assert!(inspection.summary["comments"].as_array().is_some());
    assert!(inspection.summary["charts"].as_array().is_some());
    assert!(inspection.summary["media_parts"].as_array().is_some());
}

#[test]
fn inspect_emits_empty_pictures_for_minimal() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let inspection = handler
        .inspect(&handler.parse(&path).expect("parse"))
        .expect("inspect");
    let pictures = inspection.summary["pictures"]
        .as_array()
        .expect("pictures array");
    assert!(pictures.is_empty());
}

#[test]
fn insert_picture_adds_media_and_appears_in_inspect() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");
    let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_picture".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "bytes_base64": encoded,
                    "content_type": "image/png",
                    "name": "Stamp",
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let out = directory.path().join("after.pptx");
    fs::write(&out, &patched.bytes).expect("write patched");

    let after = parse_presentation_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.pictures.len(), 1);
    assert_eq!(after.pictures[0].slide, "Slide 1");
    assert_eq!(after.pictures[0].name, "Stamp");
    assert!(after.pictures[0].part.ends_with("image1.png"));

    let entries = zip_entries(&patched.bytes);
    assert!(
        entries.contains_key("ppt/media/image1.png"),
        "media part missing"
    );
    assert_eq!(
        entries.get("ppt/media/image1.png").expect("media bytes"),
        &TINY_PNG.to_vec()
    );
    let slide = String::from_utf8_lossy(entries.get("ppt/slides/slide1.xml").expect("slide"));
    assert!(slide.contains("<p:pic>"), "expected p:pic in slide");
    assert!(slide.contains("name=\"Stamp\""), "expected picture name");
    assert_eq!(edit.semantic_diff[0].change, "insert_picture");

    let caps = handler
        .inspect(&handler.parse(&out).expect("reparse inspect"))
        .expect("inspect");
    assert!(
        caps.edit_capabilities
            .iter()
            .any(|cap| cap.operation == "insert_picture"),
        "insert_picture capability missing"
    );
}

#[test]
fn insert_picture_does_not_rewrite_existing_media_bytes() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    let before = minimal_pptx_with_media(true);
    fs::write(&path, &before).expect("write fixture");
    let existing = zip_entries(&before)
        .into_iter()
        .filter(|(name, _)| name.starts_with("ppt/media/"))
        .collect::<BTreeMap<_, _>>();
    assert!(
        existing.contains_key("ppt/media/logo.bin"),
        "expected logo.bin fixture"
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);

    let handler = PptxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "insert_picture".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "bytes_base64": encoded
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = zip_entries(&patched.bytes);
    for (name, bytes) in existing {
        assert_eq!(
            after.get(&name).expect("retained media"),
            &bytes,
            "existing media rewritten: {name}"
        );
    }
    assert!(
        after
            .keys()
            .any(|name| name.starts_with("ppt/media/image") && name.ends_with(".png")),
        "expected new image media part"
    );
}

#[test]
fn insert_picture_rejects_duplicate_name() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("logo.pptx");
    fs::write(&path, pptx_with_png()).expect("write fixture");
    let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);

    let handler = PptxFormat;
    let error = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "insert_picture".into(),
                payload: serde_json::json!({
                    "slide": "Slide 1",
                    "bytes_base64": encoded,
                    "name": "Logo",
                }),
            }],
        )
        .expect_err("duplicate name");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("already") || message.contains("duplicate") || message.contains("logo"),
        "unexpected error: {error}"
    );
}

#[test]
fn mutate_picture_kinds_are_rejected() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    for kind in ["replace_picture", "delete_picture", "set_picture"] {
        let error = handler
            .validate_edit(
                &model,
                &[SemanticOperation {
                    kind: kind.into(),
                    payload: serde_json::json!({ "element_id": "pic_x" }),
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
