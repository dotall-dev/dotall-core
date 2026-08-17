use std::fs;

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_pdf::{PdfFormat, minimal_form_pdf, minimal_text_annot_pdf, parse_pdf_bytes};
use lopdf::{Document, Object};
use tempfile::tempdir;

#[test]
fn inspect_text_annot_in_comments_excludes_widget() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("annot.pdf");
    fs::write(&path, minimal_text_annot_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    let comments = inspection.summary["comments"]
        .as_array()
        .expect("comments array");
    assert_eq!(comments.len(), 1, "{comments:?}");
    let comment = &comments[0];
    assert_eq!(comment["page"], 1);
    assert_eq!(comment["subtype"], "Text");
    assert_eq!(comment["contents"], "Check Name field");
    assert_eq!(comment["author"], "Dotall");
    assert!(
        comment["element_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("an_")),
        "element_id={}",
        comment["element_id"]
    );
    let names: Vec<&str> = comments
        .iter()
        .filter_map(|c| c["subtype"].as_str())
        .collect();
    assert!(!names.contains(&"Widget"));
}

#[test]
fn inspect_without_comment_annots_has_empty_or_omitted_comments() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    match inspection.summary.get("comments") {
        None => {}
        Some(value) => {
            let empty = value.as_array().is_some_and(|arr| arr.is_empty());
            assert!(empty, "expected omit or []; got {value}");
        }
    }
}

#[test]
fn inspect_charts_omit_or_empty() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    match inspection.summary.get("charts") {
        None => {}
        Some(value) => {
            let empty = value.as_array().is_some_and(|arr| arr.is_empty());
            assert!(empty, "expected omit or []; got {value}");
        }
    }
}

#[test]
fn insert_comment_adds_text_annot_without_changing_fields() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let before_value = model.payload["fields"][0]["value"]
        .as_str()
        .expect("Name value")
        .to_owned();

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "page": 1,
                    "contents": "Check Name field",
                    "author": "Dotall"
                }),
            }],
        )
        .expect("validate insert_comment");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_pdf_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.fields[0].value, before_value);

    let after_model = handler
        .parse({
            fs::write(&path, &patched.bytes).expect("rewrite");
            &path
        })
        .expect("reparse path");
    let inspection = handler.inspect(&after_model).expect("inspect");
    let comments = inspection.summary["comments"].as_array().expect("comments");
    assert!(
        comments.iter().any(|c| {
            c["page"] == 1
                && c["subtype"] == "Text"
                && c["contents"] == "Check Name field"
                && c["author"] == "Dotall"
        }),
        "{comments:?}"
    );
}

#[test]
fn insert_comment_preserves_contents_stream_and_existing_annots() {
    let before_bytes = minimal_form_pdf();
    let before_doc = Document::load_mem(&before_bytes).expect("load before");
    let before_contents = contents_stream_bytes(&before_doc, 1).expect("before contents");
    let before_widget = annot_dict_bytes(&before_doc, b"Widget").expect("before widget");

    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, &before_bytes).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "page": 1,
                    "contents": "Check Name field"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");

    let after_doc = Document::load_mem(&patched.bytes).expect("load after");
    let after_contents = contents_stream_bytes(&after_doc, 1).expect("after contents");
    let after_widget = annot_dict_bytes(&after_doc, b"Widget").expect("after widget");
    assert_eq!(
        before_contents, after_contents,
        "page /Contents stream must stay identical"
    );
    assert_eq!(
        before_widget, after_widget,
        "existing Widget annot dict must stay identical"
    );
}

#[test]
fn mutate_existing_comment_kinds_reject() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("annot.pdf");
    fs::write(&path, minimal_text_annot_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    for kind in [
        "set_comment",
        "delete_comment",
        "replace_comment",
        "draw_text_on_page",
    ] {
        let err = handler
            .validate_edit(
                &model,
                &[SemanticOperation {
                    kind: kind.into(),
                    payload: serde_json::json!({ "page": 1, "contents": "x" }),
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
fn insert_comment_is_advertised() {
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
            .any(|cap| cap.operation == "insert_comment"),
        "insert_comment missing from edit_capabilities"
    );
}

fn contents_stream_bytes(document: &Document, page_number: u32) -> Option<Vec<u8>> {
    let pages = document.get_pages();
    let page_id = pages.get(&page_number)?;
    let page = document.get_object(*page_id).ok()?;
    let Object::Dictionary(dict) = page else {
        return None;
    };
    let contents = dict.get(b"Contents").ok()?;
    let content_id = match contents {
        Object::Reference(id) => *id,
        _ => return None,
    };
    let object = document.get_object(content_id).ok()?;
    match object {
        Object::Stream(stream) => Some(stream.content.clone()),
        _ => None,
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
