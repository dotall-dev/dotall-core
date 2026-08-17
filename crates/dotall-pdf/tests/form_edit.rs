use std::fs;
use std::sync::Arc;

use dotall_core::registry::{FormatHandler, FormatRegistry};
use dotall_core::{Actor, ActorKind, DotallStore, EditRequest, Engine, SemanticOperation};
use dotall_pdf::{
    PdfFormat, demo_form_pdf, minimal_checkbox_pdf, minimal_form_pdf, minimal_radio_pdf,
    parse_pdf_bytes,
};
use tempfile::tempdir;

#[test]
fn set_form_field_updates_radio_group_export_value() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("radio.pdf");
    fs::write(&path, minimal_radio_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let field = model
        .payload
        .get("fields")
        .and_then(|fields| fields.as_array())
        .and_then(|fields| fields.iter().find(|f| f["name"] == "Priority"))
        .expect("Priority radio");
    assert_eq!(field["field_type"], "btn");
    assert_eq!(field["value"], "Medium");
    let export_values = field["export_values"]
        .as_array()
        .expect("export_values")
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    assert!(export_values.contains(&"Low"));
    assert!(export_values.contains(&"Medium"));
    assert!(export_values.contains(&"High"));
    assert!(export_values.contains(&"Off"));

    let ambiguous = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field".into(),
                payload: serde_json::json!({ "name": "Priority", "value": "On" }),
            }],
        )
        .expect_err("On is ambiguous for radio");
    assert!(
        ambiguous.to_string().contains("ambiguous radio"),
        "{ambiguous}"
    );

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field".into(),
                payload: serde_json::json!({ "name": "Priority", "value": "High" }),
            }],
        )
        .expect("validate radio");
    let patched = handler.apply_edit(&path, &edit).expect("apply radio");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    let priority = after
        .fields
        .iter()
        .find(|f| f.name == "Priority")
        .expect("Priority");
    assert_eq!(priority.value, "High");
}

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
fn clear_form_field_blanks_text_and_turns_checkbox_off() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let filled = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field".into(),
                payload: serde_json::json!({ "name": "Name", "value": "Grace" }),
            }],
        )
        .expect("validate fill");
    let after_fill = handler.apply_edit(&path, &filled).expect("apply fill");
    fs::write(&path, &after_fill.bytes).expect("rewrite");

    let model = handler.parse(&path).expect("reparse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "clear_form_field".into(),
                payload: serde_json::json!({ "name": "Name" }),
            }],
        )
        .expect("validate clear");
    let patched = handler.apply_edit(&path, &edit).expect("apply clear");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse cleared");
    let name = after
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    assert_eq!(name.value, "");
    assert_eq!(edit.semantic_diff[0].change, "clear_form_field");
}

#[test]
fn clear_form_field_sets_checkbox_off() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("checkbox.pdf");
    fs::write(&path, minimal_checkbox_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let on = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field".into(),
                payload: serde_json::json!({ "name": "Agree", "value": "On" }),
            }],
        )
        .expect("validate on");
    let after_on = handler.apply_edit(&path, &on).expect("apply on");
    fs::write(&path, &after_on.bytes).expect("rewrite");

    let model = handler.parse(&path).expect("reparse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "clear_form_field".into(),
                payload: serde_json::json!({ "name": "Agree" }),
            }],
        )
        .expect("validate clear");
    let patched = handler.apply_edit(&path, &edit).expect("apply clear");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    let agree = after
        .fields
        .iter()
        .find(|field| field.name == "Agree")
        .expect("Agree");
    assert_eq!(agree.value, "Off");
}

#[test]
fn clear_form_field_sets_radio_off() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("radio.pdf");
    fs::write(&path, minimal_radio_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "clear_form_field".into(),
                payload: serde_json::json!({ "name": "Priority" }),
            }],
        )
        .expect("validate clear radio");
    let patched = handler.apply_edit(&path, &edit).expect("apply clear radio");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    let priority = after
        .fields
        .iter()
        .find(|field| field.name == "Priority")
        .expect("Priority");
    assert_eq!(priority.value, "Off");
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
fn set_document_metadata_updates_info_dict() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    assert_eq!(model.payload["metadata"]["title"], "Vendor Intake Form");
    assert_eq!(model.payload["metadata"]["author"], "Dotall Demo");

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_document_metadata".into(),
                payload: serde_json::json!({
                    "title": "Updated Intake",
                    "author": "Wave 4 Agent",
                    "subject": "Onboarding refresh"
                }),
            }],
        )
        .expect("validate metadata");
    let patched = handler.apply_edit(&path, &edit).expect("apply metadata");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.metadata.title.as_deref(), Some("Updated Intake"));
    assert_eq!(after.metadata.author.as_deref(), Some("Wave 4 Agent"));
    assert_eq!(
        after.metadata.subject.as_deref(),
        Some("Onboarding refresh")
    );
    // Creator / Producer from Wave 2 inspect stay untouched when omitted.
    assert_eq!(after.metadata.creator.as_deref(), Some("dotall-pdf"));
    assert_eq!(after.metadata.producer.as_deref(), Some("dotall-pdf"));
}

#[test]
fn set_document_metadata_creates_info_when_missing() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    assert!(model.payload["metadata"]["title"].is_null());

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_document_metadata".into(),
                payload: serde_json::json!({ "title": "Hello Form" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.metadata.title.as_deref(), Some("Hello Form"));
    assert_eq!(after.fields[0].value, "Ada");
}

#[test]
fn set_document_metadata_requires_at_least_one_field() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");
    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_document_metadata".into(),
                payload: serde_json::json!({}),
            }],
        )
        .expect_err("empty payload");
    assert!(error.to_string().contains("title"), "{error}");
}

#[test]
fn clear_document_metadata_clears_title_author_subject() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    assert_eq!(model.payload["metadata"]["title"], "Vendor Intake Form");
    assert_eq!(model.payload["metadata"]["author"], "Dotall Demo");

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "clear_document_metadata".into(),
                payload: serde_json::json!({}),
            }],
        )
        .expect("validate clear metadata");
    let patched = handler.apply_edit(&path, &edit).expect("apply clear");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    assert!(after.metadata.title.is_none() || after.metadata.title.as_deref() == Some(""));
    assert!(after.metadata.author.is_none() || after.metadata.author.as_deref() == Some(""));
    assert!(after.metadata.subject.is_none() || after.metadata.subject.as_deref() == Some(""));
    // Creator / Producer stay when we only clear Title/Author/Subject.
    assert_eq!(after.metadata.creator.as_deref(), Some("dotall-pdf"));
    assert_eq!(edit.semantic_diff[0].change, "clear_document_metadata");
}

#[test]
fn capabilities_advertise_clear_document_metadata() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");
    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "clear_document_metadata"),
        "capabilities must advertise clear_document_metadata"
    );
}

#[test]
fn clear_all_form_fields_clears_text_and_checkbox() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    assert_eq!(
        model.payload["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|field| field["name"] == "Name")
            .map(|field| field["value"].as_str().unwrap_or("")),
        Some("Ada Lovelace")
    );

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "clear_all_form_fields".into(),
                payload: serde_json::json!({}),
            }],
        )
        .expect("validate clear all");
    let patched = handler.apply_edit(&path, &edit).expect("apply clear all");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    let name = after
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    let agree = after
        .fields
        .iter()
        .find(|field| field.name == "Agree")
        .expect("Agree");
    assert_eq!(name.value, "");
    assert_eq!(agree.value, "Off");
    assert_eq!(edit.semantic_diff[0].change, "clear_all_form_fields");
}

#[test]
fn capabilities_advertise_clear_all_form_fields() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");
    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "clear_all_form_fields"),
        "capabilities must advertise clear_all_form_fields"
    );
}

#[test]
fn set_form_fields_sets_multiple_fields_in_one_op() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_fields".into(),
                payload: serde_json::json!({
                    "fields": {
                        "Name": "Grace Hopper",
                        "Agree": "On"
                    }
                }),
            }],
        )
        .expect("validate set_form_fields");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    let name = after
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    let agree = after
        .fields
        .iter()
        .find(|field| field.name == "Agree")
        .expect("Agree");
    assert_eq!(name.value, "Grace Hopper");
    assert_eq!(agree.value, "Yes");
    assert_eq!(edit.semantic_diff[0].change, "set_form_fields");
}

#[test]
fn capabilities_advertise_set_form_fields() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");
    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "set_form_fields"),
        "capabilities must advertise set_form_fields"
    );
}

#[test]
fn set_form_field_readonly_toggles_ff_and_blocks_value_edits() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let name_field = model
        .payload
        .get("fields")
        .and_then(|fields| fields.as_array())
        .and_then(|fields| fields.iter().find(|f| f["name"] == "Name"))
        .expect("Name");
    assert_eq!(name_field["read_only"], false);

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_readonly".into(),
                payload: serde_json::json!({ "name": "Name", "readonly": true }),
            }],
        )
        .expect("validate readonly");
    let patched = handler.apply_edit(&path, &edit).expect("apply readonly");
    fs::write(&path, &patched.bytes).expect("rewrite");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    let name = after
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    assert!(name.read_only);
    assert_eq!(edit.semantic_diff[0].change, "set_form_field_readonly");

    let model = handler.parse(&path).expect("parse after");
    let inspection = handler.inspect(&model).expect("inspect");
    let inspect_name = inspection.summary["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .find(|field| field["name"] == "Name")
        .expect("Name in inspect");
    assert_eq!(inspect_name["read_only"], true);

    let blocked = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field".into(),
                payload: serde_json::json!({ "name": "Name", "value": "Blocked" }),
            }],
        )
        .expect_err("read-only blocks value edits");
    assert!(
        blocked.to_string().to_lowercase().contains("read-only")
            || blocked.to_string().to_lowercase().contains("readonly"),
        "{blocked}"
    );

    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_readonly".into(),
                payload: serde_json::json!({ "name": "Name", "readonly": false }),
            }],
        )
        .expect("validate clear readonly");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let final_doc = parse_pdf_bytes(&cleared.bytes).expect("reparse");
    let name = final_doc
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    assert!(!name.read_only);
}

#[test]
fn set_form_field_required_toggles_ff_and_surfaces_in_inspect() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let name_field = model
        .payload
        .get("fields")
        .and_then(|fields| fields.as_array())
        .and_then(|fields| fields.iter().find(|f| f["name"] == "Name"))
        .expect("Name");
    assert_eq!(name_field["required"], false);

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_required".into(),
                payload: serde_json::json!({ "name": "Name", "required": true }),
            }],
        )
        .expect("validate required");
    let patched = handler.apply_edit(&path, &edit).expect("apply required");
    fs::write(&path, &patched.bytes).expect("rewrite");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    let name = after
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    assert!(name.required);
    assert_eq!(edit.semantic_diff[0].change, "set_form_field_required");

    let model = handler.parse(&path).expect("parse after");
    let inspection = handler.inspect(&model).expect("inspect");
    let inspect_name = inspection.summary["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .find(|field| field["name"] == "Name")
        .expect("Name in inspect");
    assert_eq!(inspect_name["required"], true);

    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_required".into(),
                payload: serde_json::json!({ "name": "Name", "required": false }),
            }],
        )
        .expect("validate clear required");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let final_doc = parse_pdf_bytes(&cleared.bytes).expect("reparse");
    let name = final_doc
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    assert!(!name.required);
}

#[test]
fn capabilities_advertise_set_form_field_required() {
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
            .any(|cap| cap.operation == "set_form_field_required"),
        "capabilities must advertise set_form_field_required"
    );
}

#[test]
fn set_form_field_multiline_toggles_ff_and_surfaces_in_inspect() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let name_field = model
        .payload
        .get("fields")
        .and_then(|fields| fields.as_array())
        .and_then(|fields| fields.iter().find(|f| f["name"] == "Name"))
        .expect("Name");
    assert_eq!(name_field["multiline"], false);

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_multiline".into(),
                payload: serde_json::json!({ "name": "Name", "multiline": true }),
            }],
        )
        .expect("validate multiline");
    let patched = handler.apply_edit(&path, &edit).expect("apply multiline");
    fs::write(&path, &patched.bytes).expect("rewrite");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    let name = after
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    assert!(name.multiline);
    assert_eq!(edit.semantic_diff[0].change, "set_form_field_multiline");

    let model = handler.parse(&path).expect("parse after");
    let inspection = handler.inspect(&model).expect("inspect");
    let inspect_name = inspection.summary["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .find(|field| field["name"] == "Name")
        .expect("Name in inspect");
    assert_eq!(inspect_name["multiline"], true);

    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_multiline".into(),
                payload: serde_json::json!({ "name": "Name", "multiline": false }),
            }],
        )
        .expect("validate clear multiline");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let final_doc = parse_pdf_bytes(&cleared.bytes).expect("reparse");
    let name = final_doc
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    assert!(!name.multiline);
}

#[test]
fn set_form_field_multiline_rejects_non_text() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    // Prefer a known non-tx field from the fixture when present.
    let non_tx = model
        .payload
        .get("fields")
        .and_then(|fields| fields.as_array())
        .and_then(|fields| {
            fields.iter().find(|f| {
                f["field_type"] == "btn" || f["field_type"] == "ch" || f["field_type"] == "sig"
            })
        });
    let Some(field) = non_tx else {
        return;
    };
    let name = field["name"].as_str().expect("name");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_multiline".into(),
                payload: serde_json::json!({ "name": name, "multiline": true }),
            }],
        )
        .expect_err("non-text");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("text") || message.contains("multiline") || message.contains("tx"),
        "unexpected error: {error}"
    );
}

#[test]
fn capabilities_advertise_set_form_field_multiline() {
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
            .any(|cap| cap.operation == "set_form_field_multiline"),
        "capabilities must advertise set_form_field_multiline"
    );
}

#[test]
fn set_form_field_password_toggles_ff_and_surfaces_in_inspect() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let name_field = model
        .payload
        .get("fields")
        .and_then(|fields| fields.as_array())
        .and_then(|fields| fields.iter().find(|f| f["name"] == "Name"))
        .expect("Name");
    assert_eq!(name_field["password"], false);

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_password".into(),
                payload: serde_json::json!({ "name": "Name", "password": true }),
            }],
        )
        .expect("validate password");
    let patched = handler.apply_edit(&path, &edit).expect("apply password");
    fs::write(&path, &patched.bytes).expect("rewrite");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    let name = after
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    assert!(name.password);
    assert_eq!(edit.semantic_diff[0].change, "set_form_field_password");

    let model = handler.parse(&path).expect("parse after");
    let inspection = handler.inspect(&model).expect("inspect");
    let inspect_name = inspection.summary["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .find(|field| field["name"] == "Name")
        .expect("Name in inspect");
    assert_eq!(inspect_name["password"], true);

    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_password".into(),
                payload: serde_json::json!({ "name": "Name", "password": false }),
            }],
        )
        .expect("validate clear password");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let final_doc = parse_pdf_bytes(&cleared.bytes).expect("reparse");
    let name = final_doc
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    assert!(!name.password);
}

#[test]
fn set_form_field_password_rejects_non_text() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let non_tx = model
        .payload
        .get("fields")
        .and_then(|fields| fields.as_array())
        .and_then(|fields| {
            fields.iter().find(|f| {
                f["field_type"] == "btn" || f["field_type"] == "ch" || f["field_type"] == "sig"
            })
        });
    let Some(field) = non_tx else {
        return;
    };
    let name = field["name"].as_str().expect("name");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_password".into(),
                payload: serde_json::json!({ "name": name, "password": true }),
            }],
        )
        .expect_err("non-text");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("text") || message.contains("password") || message.contains("tx"),
        "unexpected error: {error}"
    );
}

#[test]
fn capabilities_advertise_set_form_field_password() {
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
            .any(|cap| cap.operation == "set_form_field_password"),
        "capabilities must advertise set_form_field_password"
    );
}

#[test]
fn set_form_field_max_length_sets_and_clears_maxlen() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let name_field = model
        .payload
        .get("fields")
        .and_then(|fields| fields.as_array())
        .and_then(|fields| fields.iter().find(|f| f["name"] == "Name"))
        .expect("Name");
    assert!(name_field.get("max_length").is_none() || name_field["max_length"].is_null());

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_max_length".into(),
                payload: serde_json::json!({ "name": "Name", "max_length": 32 }),
            }],
        )
        .expect("validate max_length");
    let patched = handler.apply_edit(&path, &edit).expect("apply max_length");
    fs::write(&path, &patched.bytes).expect("rewrite");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    let name = after
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    assert_eq!(name.max_length, Some(32));
    assert_eq!(edit.semantic_diff[0].change, "set_form_field_max_length");

    let model = handler.parse(&path).expect("parse after");
    let inspection = handler.inspect(&model).expect("inspect");
    let inspect_name = inspection.summary["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .find(|field| field["name"] == "Name")
        .expect("Name in inspect");
    assert_eq!(inspect_name["max_length"], 32);

    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_max_length".into(),
                payload: serde_json::json!({ "name": "Name", "max_length": null }),
            }],
        )
        .expect("validate clear max_length");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let final_doc = parse_pdf_bytes(&cleared.bytes).expect("reparse");
    let name = final_doc
        .fields
        .iter()
        .find(|field| field.name == "Name")
        .expect("Name");
    assert_eq!(name.max_length, None);
}

#[test]
fn set_form_field_max_length_rejects_non_text() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let non_tx = model
        .payload
        .get("fields")
        .and_then(|fields| fields.as_array())
        .and_then(|fields| {
            fields.iter().find(|f| {
                f["field_type"] == "btn" || f["field_type"] == "ch" || f["field_type"] == "sig"
            })
        });
    let Some(field) = non_tx else {
        return;
    };
    let name = field["name"].as_str().expect("name");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_field_max_length".into(),
                payload: serde_json::json!({ "name": name, "max_length": 10 }),
            }],
        )
        .expect_err("non-text");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("text") || message.contains("max_length") || message.contains("tx"),
        "unexpected error: {error}"
    );
}

#[test]
fn capabilities_advertise_set_form_field_max_length() {
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
            .any(|cap| cap.operation == "set_form_field_max_length"),
        "capabilities must advertise set_form_field_max_length"
    );
}

#[test]
fn capabilities_advertise_set_form_field_readonly() {
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
            .any(|cap| cap.operation == "set_form_field_readonly"),
        "capabilities must advertise set_form_field_readonly"
    );
}

#[test]
fn set_form_fields_rejects_empty_map() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");
    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_form_fields".into(),
                payload: serde_json::json!({ "fields": {} }),
            }],
        )
        .expect_err("empty fields");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("field") || message.contains("empty") || message.contains("required"),
        "unexpected error: {error}"
    );
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

#[test]
fn revert_after_radio_fill_restores_original_bytes() {
    let workspace = tempdir().expect("workspace");
    let source = workspace.path().join("radio.pdf");
    let original = minimal_radio_pdf();
    fs::write(&source, &original).expect("write fixture");

    let store = DotallStore::init(workspace.path()).expect("store");
    let mut registry = FormatRegistry::default();
    registry.register(Arc::new(PdfFormat));
    let mut engine = Engine::new(store, registry);

    let hash = engine.load_model("radio.pdf").expect("load").source_hash;
    engine
        .edit(
            "radio.pdf",
            &EditRequest {
                transaction_id: uuid(5),
                expected_source_hash: hash,
                actor: Actor {
                    kind: ActorKind::Cli,
                    id: Some("pdf-radio".into()),
                },
                operations: vec![SemanticOperation {
                    kind: "set_form_field".into(),
                    payload: serde_json::json!({ "name": "Priority", "value": "High" }),
                }],
            },
        )
        .expect("stage");
    engine.apply("radio.pdf", uuid(5)).expect("apply");
    let after_apply = parse_pdf_bytes(&fs::read(&source).expect("read after")).expect("parse");
    let priority = after_apply
        .fields
        .iter()
        .find(|f| f.name == "Priority")
        .expect("Priority");
    assert_eq!(priority.value, "High");
    assert_ne!(fs::read(&source).expect("after apply"), original);

    engine
        .revert("radio.pdf", 1, uuid(6))
        .expect("stage revert");
    engine.apply("radio.pdf", uuid(6)).expect("apply revert");
    assert_eq!(fs::read(&source).expect("restored"), original);
}

fn uuid(n: u8) -> uuid::Uuid {
    uuid::Uuid::parse_str(&format!("00000000-0000-0000-0000-00000000000{n}")).expect("uuid")
}
