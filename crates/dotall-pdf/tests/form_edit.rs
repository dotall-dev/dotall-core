use std::fs;
use std::sync::Arc;

use dotall_core::registry::{FormatHandler, FormatRegistry};
use dotall_core::{Actor, ActorKind, DotallStore, EditRequest, Engine, SemanticOperation};
use dotall_pdf::{PdfFormat, minimal_checkbox_pdf, minimal_form_pdf, parse_pdf_bytes};
use tempfile::tempdir;

#[test]
fn set_form_field_updates_checkbox_on() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("checkbox.pdf");
    fs::write(&path, minimal_checkbox_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let field = model
        .payload
        .get("fields")
        .and_then(|fields| fields.as_array())
        .and_then(|fields| fields.first())
        .expect("checkbox field");
    assert_eq!(field["name"], "Agree");
    assert_eq!(field["field_type"], "btn");
    assert_eq!(field["value"], "Off");
    let export_values = field["export_values"]
        .as_array()
        .expect("export_values")
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    assert!(export_values.contains(&"Yes"));
    assert!(export_values.contains(&"Off"));

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field".into(),
                payload: serde_json::json!({ "name": "Agree", "value": "On" }),
            }],
        )
        .expect("validate checkbox");
    let patched = handler.apply_edit(&path, &edit).expect("apply checkbox");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.fields[0].name, "Agree");
    assert_eq!(after.fields[0].value, "Yes");
}

#[test]
fn set_form_field_updates_value() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field".into(),
                payload: serde_json::json!({ "name": "Name", "value": "Grace" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.fields[0].value, "Grace");
}

#[test]
fn rejects_unknown_field() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");
    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field".into(),
                payload: serde_json::json!({ "name": "Missing", "value": "x" }),
            }],
        )
        .expect_err("unknown field");
    assert!(error.to_string().contains("was not found"));
}

#[test]
fn revert_after_form_fill_restores_original_bytes() {
    let workspace = tempdir().expect("workspace");
    let source = workspace.path().join("form.pdf");
    let original = minimal_form_pdf();
    fs::write(&source, &original).expect("write fixture");

    let store = DotallStore::init(workspace.path()).expect("store");
    let mut registry = FormatRegistry::default();
    registry.register(Arc::new(PdfFormat));
    let mut engine = Engine::new(store, registry);

    let hash = engine.load_model("form.pdf").expect("load").source_hash;
    engine
        .edit(
            "form.pdf",
            &EditRequest {
                transaction_id: uuid(1),
                expected_source_hash: hash,
                actor: Actor {
                    kind: ActorKind::Cli,
                    id: Some("pdf-test".into()),
                },
                operations: vec![SemanticOperation {
                    kind: "set_form_field".into(),
                    payload: serde_json::json!({ "name": "Name", "value": "Grace" }),
                }],
            },
        )
        .expect("stage");
    engine.apply("form.pdf", uuid(1)).expect("apply");
    assert_ne!(fs::read(&source).expect("after apply"), original);

    engine.revert("form.pdf", 1, uuid(2)).expect("stage revert");
    engine.apply("form.pdf", uuid(2)).expect("apply revert");
    assert_eq!(fs::read(&source).expect("restored"), original);
}

#[test]
fn revert_after_checkbox_fill_restores_original_bytes() {
    let workspace = tempdir().expect("workspace");
    let source = workspace.path().join("checkbox.pdf");
    let original = minimal_checkbox_pdf();
    fs::write(&source, &original).expect("write fixture");

    let store = DotallStore::init(workspace.path()).expect("store");
    let mut registry = FormatRegistry::default();
    registry.register(Arc::new(PdfFormat));
    let mut engine = Engine::new(store, registry);

    let hash = engine.load_model("checkbox.pdf").expect("load").source_hash;
    engine
        .edit(
            "checkbox.pdf",
            &EditRequest {
                transaction_id: uuid(3),
                expected_source_hash: hash,
                actor: Actor {
                    kind: ActorKind::Cli,
                    id: Some("pdf-checkbox".into()),
                },
                operations: vec![SemanticOperation {
                    kind: "set_form_field".into(),
                    payload: serde_json::json!({ "name": "Agree", "value": "On" }),
                }],
            },
        )
        .expect("stage");
    engine.apply("checkbox.pdf", uuid(3)).expect("apply");
    let after_apply = parse_pdf_bytes(&fs::read(&source).expect("read after")).expect("parse");
    assert_eq!(after_apply.fields[0].value, "Yes");
    assert_ne!(fs::read(&source).expect("after apply"), original);

    engine
        .revert("checkbox.pdf", 1, uuid(4))
        .expect("stage revert");
    engine.apply("checkbox.pdf", uuid(4)).expect("apply revert");
    assert_eq!(fs::read(&source).expect("restored"), original);
}

fn uuid(n: u8) -> uuid::Uuid {
    uuid::Uuid::parse_str(&format!("00000000-0000-0000-0000-00000000000{n}")).expect("uuid")
}
