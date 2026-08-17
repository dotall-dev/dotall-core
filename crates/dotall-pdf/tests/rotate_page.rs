use std::fs;

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_pdf::{PdfFormat, minimal_form_pdf, parse_pdf_bytes};
use tempfile::tempdir;

#[test]
fn rotate_page_sets_rotate_90() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let before_name = model.payload["fields"]
        .as_array()
        .and_then(|fields| fields.iter().find(|f| f["name"] == "Name"))
        .and_then(|f| f["value"].as_str())
        .expect("Name field")
        .to_owned();
    assert_eq!(before_name, "Ada");

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "rotate_page".into(),
                payload: serde_json::json!({ "page": 1, "degrees": 90 }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.pages[0].rotate, Some(90));
    assert_eq!(after.fields[0].value, "Ada");

    fs::write(&path, &patched.bytes).expect("rewrite");
    let inspection = handler
        .inspect(&handler.parse(&path).expect("parse"))
        .expect("inspect");
    assert_eq!(inspection.summary["page_count"], 1);
    assert_eq!(inspection.summary["page_rotations"][0]["page"], 1);
    assert_eq!(inspection.summary["page_rotations"][0]["rotate"], 90);
    let name_field = inspection.summary["fields"]
        .as_array()
        .and_then(|fields| fields.iter().find(|f| f["name"] == "Name"))
        .expect("Name in inspect");
    assert_eq!(name_field["value"], "Ada");
}

#[test]
fn rotate_page_zero_clears_rotate() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write");

    let handler = PdfFormat;
    let edit_90 = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "rotate_page".into(),
                payload: serde_json::json!({ "page": 1, "degrees": 90 }),
            }],
        )
        .expect("validate 90");
    let patched_90 = handler.apply_edit(&path, &edit_90).expect("apply 90");
    fs::write(&path, &patched_90.bytes).expect("rewrite 90");

    let edit_0 = handler
        .validate_edit(
            &handler.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "rotate_page".into(),
                payload: serde_json::json!({ "page": 1, "degrees": 0 }),
            }],
        )
        .expect("validate 0");
    let patched_0 = handler.apply_edit(&path, &edit_0).expect("apply 0");
    let after = parse_pdf_bytes(&patched_0.bytes).expect("reparse");
    assert_eq!(after.pages[0].rotate, None);
    assert_eq!(after.fields[0].value, "Ada");

    fs::write(&path, &patched_0.bytes).expect("rewrite 0");
    let inspection = handler
        .inspect(&handler.parse(&path).expect("parse"))
        .expect("inspect");
    assert_eq!(inspection.summary["page_count"], 1);
    let rotations = inspection.summary["page_rotations"]
        .as_array()
        .expect("page_rotations");
    assert!(
        rotations.is_empty()
            || rotations
                .iter()
                .all(|entry| entry["rotate"].as_u64() == Some(0)),
        "expected no non-zero rotations after clear: {rotations:?}"
    );
}

#[test]
fn rotate_page_rejects_45_degrees() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write");
    let error = PdfFormat
        .validate_edit(
            &PdfFormat.parse(&path).expect("parse"),
            &[SemanticOperation {
                kind: "rotate_page".into(),
                payload: serde_json::json!({ "page": 1, "degrees": 45 }),
            }],
        )
        .expect_err("45");
    assert!(
        error.to_string().contains("90") || error.to_string().contains("degrees"),
        "got {error}"
    );
}

#[test]
fn rotate_page_is_advertised() {
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
            .any(|cap| cap.operation == "rotate_page"),
        "rotate_page missing from edit_capabilities"
    );
}

#[test]
fn rotate_page_rejects_encrypted_pdf() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("encrypted.pdf");
    fs::write(&path, b"%PDF-1.4\n/Encrypt\n").expect("write stub");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse encrypted stub");
    assert!(model.payload["encrypted"].as_bool().unwrap_or(false));

    let err = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "rotate_page".into(),
                payload: serde_json::json!({ "page": 1, "degrees": 90 }),
            }],
        )
        .expect_err("encrypted");
    assert!(
        err.to_string().contains("encrypted"),
        "expected encrypted rejection, got {err}"
    );
}
