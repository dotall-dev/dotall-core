use std::fs;

use base64::Engine;
use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_pdf::{PdfFormat, minimal_form_pdf, minimal_stamp_annot_pdf, parse_pdf_bytes};
use lopdf::{Document, Object};
use tempfile::tempdir;

/// 1×1 8-bit RGB PNG (red pixel) — shared Office-parity atlas fixture.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[test]
fn inspect_stamp_annot_in_pictures_not_comments() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("stamp.pdf");
    fs::write(&path, minimal_stamp_annot_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");

    let pictures = inspection.summary["pictures"]
        .as_array()
        .expect("pictures array");
    assert_eq!(pictures.len(), 1, "{pictures:?}");
    let picture = &pictures[0];
    assert_eq!(picture["page"], 1);
    assert_eq!(picture["subtype"], "Stamp");
    assert!(
        picture["element_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("an_")),
        "element_id={}",
        picture["element_id"]
    );

    let comments = inspection.summary["comments"]
        .as_array()
        .expect("comments array");
    assert!(
        comments
            .iter()
            .all(|c| c["subtype"].as_str() != Some("Stamp")),
        "Stamp must not appear in comments: {comments:?}"
    );
    assert!(
        comments.iter().any(|c| c["subtype"] == "Text"),
        "Text stays in comments: {comments:?}"
    );
}

#[test]
fn inspect_without_stamps_emits_empty_pictures() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    let pictures = inspection.summary["pictures"]
        .as_array()
        .expect("pictures must always be present");
    assert!(pictures.is_empty(), "{pictures:?}");
    match inspection.summary.get("charts") {
        None => {}
        Some(value) => {
            let empty = value.as_array().is_some_and(|arr| arr.is_empty());
            assert!(empty, "expected omit or []; got {value}");
        }
    }
}

#[test]
fn insert_picture_adds_stamp_without_rewriting_contents_or_widgets() {
    let before_bytes = minimal_form_pdf();
    let before_contents = page_contents_bytes(&before_bytes);
    let before_doc = Document::load_mem(&before_bytes).expect("load before");
    let before_widget = annot_dict_bytes(&before_doc, b"Widget").expect("before widget");

    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, &before_bytes).expect("write fixture");

    let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);
    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_picture".into(),
                payload: serde_json::json!({
                    "page": 1,
                    "bytes_base64": encoded,
                    "content_type": "image/png",
                    "rect": [10, 150, 50, 190]
                }),
            }],
        )
        .expect("validate insert_picture");
    let patched = handler.apply_edit(&path, &edit).expect("apply");

    let after_contents = page_contents_bytes(&patched.bytes);
    assert_eq!(
        before_contents, after_contents,
        "page /Contents stream must stay identical"
    );

    let after_doc = Document::load_mem(&patched.bytes).expect("load after");
    let after_widget = annot_dict_bytes(&after_doc, b"Widget").expect("after widget");
    assert_eq!(
        before_widget, after_widget,
        "existing Widget annot dict must stay identical"
    );

    let stamp = annot_dict_bytes(&after_doc, b"Stamp").expect("stamp annot");
    let stamp_text = String::from_utf8_lossy(&stamp);
    assert!(
        stamp_text.contains("Stamp") || stamp_text.contains("\"Stamp\""),
        "stamp dict present: {stamp_text}"
    );

    fs::write(&path, &patched.bytes).expect("rewrite");
    let after_model = handler.parse(&path).expect("reparse");
    let inspection = handler.inspect(&after_model).expect("inspect");
    let pictures = inspection.summary["pictures"].as_array().expect("pictures");
    assert!(
        pictures
            .iter()
            .any(|p| p["page"] == 1 && p["subtype"] == "Stamp"),
        "{pictures:?}"
    );

    let after = parse_pdf_bytes(&patched.bytes).expect("reparse bytes");
    assert_eq!(after.fields[0].value, "Ada");
}

#[test]
fn insert_picture_is_advertised() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "insert_picture"),
        "insert_picture missing from edit_capabilities"
    );
}

#[test]
fn mutate_and_draw_picture_kinds_reject() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("stamp.pdf");
    fs::write(&path, minimal_stamp_annot_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    for kind in ["draw_image", "replace_picture", "delete_picture"] {
        let err = handler
            .validate_edit(
                &model,
                &[SemanticOperation {
                    kind: kind.into(),
                    payload: serde_json::json!({ "page": 1 }),
                }],
            )
            .expect_err(kind);
        assert!(
            err.to_string().contains("unsupported") || err.to_string().contains(kind),
            "{kind}: {err}"
        );
    }
}

#[test]
fn insert_picture_rejects_encrypted_pdf() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("encrypted.pdf");
    fs::write(&path, b"%PDF-1.4\n/Encrypt\n").expect("write stub");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse encrypted stub");
    assert!(model.payload["encrypted"].as_bool().unwrap_or(false));

    let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);
    let err = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_picture".into(),
                payload: serde_json::json!({
                    "page": 1,
                    "bytes_base64": encoded
                }),
            }],
        )
        .expect_err("encrypted");
    assert!(
        err.to_string().contains("encrypted"),
        "expected encrypted rejection, got {err}"
    );
}

/// Helper copied from the Wave 23 PDF tests section of the Office-parity atlas plan.
fn page_contents_bytes(package: &[u8]) -> Vec<u8> {
    let document = Document::load_mem(package).expect("pdf");
    let pages = document.get_pages();
    let page_id = *pages.get(&1).expect("page 1");
    let page = document
        .get_object(page_id)
        .expect("page obj")
        .as_dict()
        .expect("dict");
    let contents = page.get(b"Contents").expect("Contents");
    match contents {
        Object::Reference(id) => document
            .get_object(*id)
            .expect("stream")
            .as_stream()
            .expect("stream")
            .content
            .clone(),
        other => panic!("unexpected Contents: {other:?}"),
    }
}

fn annot_dict_bytes(document: &Document, subtype: &[u8]) -> Option<Vec<u8>> {
    for object in document.objects.values() {
        let Object::Dictionary(dict) = object else {
            continue;
        };
        let Ok(Object::Name(name)) = dict.get(b"Subtype") else {
            continue;
        };
        if name.as_slice() != subtype {
            continue;
        }
        return Some(format!("{dict:?}").into_bytes());
    }
    None
}
