use std::fs;

use dotall_core::registry::{FormatHandler, ReadRequest, ReadSelector};
use dotall_docx::{DocxFormat, docx_with_comment, minimal_docx, table_docx};
use tempfile::tempdir;

fn write_fixture(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join(name);
    fs::write(&path, bytes).expect("write fixture");
    (directory, path)
}

#[test]
fn inspect_lists_headings_and_suggests_paragraph_reads() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");

    assert_eq!(inspection.format_id, "docx");
    assert_eq!(inspection.summary["paragraph_count"], 2);
    assert_eq!(inspection.summary["headings"][0]["text"], "Alpha");
    assert_eq!(inspection.summary["skipped_tables"], false);
    assert_eq!(inspection.summary["table_count"], 0);
    assert_eq!(inspection.suggested_reads[0].selector.kind, "paragraphs");
}

#[test]
fn read_paragraphs_returns_body_text() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let response = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "paragraphs".into(),
                    value: "0:2".into(),
                }),
                max_tokens: 2_000,
                continuation: None,
            },
        )
        .expect("read");

    assert!(response.content.contains("Alpha"));
    assert!(response.content.contains("Beta"));
}

#[test]
fn inspect_and_read_include_table_cell_paragraphs_in_document_order() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("table.docx");
    fs::write(&path, table_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");

    assert_eq!(inspection.summary["paragraph_count"], 3);
    assert_eq!(inspection.summary["table_count"], 1);
    assert_eq!(inspection.summary["skipped_tables"], false);

    let response = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "paragraphs".into(),
                    value: "0:3".into(),
                }),
                max_tokens: 2_000,
                continuation: None,
            },
        )
        .expect("read");

    assert!(response.content.contains("0. Intro"));
    assert!(response.content.contains("1. CellA"));
    assert!(response.content.contains("2. CellB"));
}

#[test]
fn inspect_and_read_include_header_paragraphs() {
    let (_directory, path) = write_fixture("table.docx", &table_docx());

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");

    assert_eq!(inspection.summary["header_paragraph_count"], 1);
    assert_eq!(inspection.summary["headers"][0]["text"], "HeaderOnly");
    assert_eq!(inspection.summary["headers"][0]["part"], "header1");
    assert_eq!(inspection.summary["headers"][0]["index"], 0);
    // Body indices stay document-order only (no header bleed).
    assert_eq!(inspection.summary["paragraph_count"], 3);

    let response = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "headers".into(),
                    value: "0:1".into(),
                }),
                max_tokens: 2_000,
                continuation: None,
            },
        )
        .expect("read headers");

    assert!(response.content.contains("header1"));
    assert!(response.content.contains("HeaderOnly"));
}

#[test]
fn inspect_lists_comments_with_paragraph_element_id() {
    let (_directory, path) = write_fixture("commented.docx", &docx_with_comment());

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    let paragraph_id = model.payload["paragraphs"][1]["element_id"]
        .as_str()
        .expect("paragraph element_id");

    let comments = inspection.summary["comments"]
        .as_array()
        .expect("comments array");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["author"], "Ada");
    assert_eq!(comments[0]["text"], "Confirm owners");
    assert_eq!(comments[0]["index"], 1);
    assert_eq!(comments[0]["paragraph"], paragraph_id);
    let element_id = comments[0]["element_id"]
        .as_str()
        .expect("comment element_id");
    assert!(
        element_id.starts_with("cm_"),
        "expected opaque cm_ id, got {element_id}"
    );
}

#[test]
fn inspect_omits_or_empties_comments_and_charts_when_absent() {
    let (_directory, path) = write_fixture("memo.docx", &minimal_docx());

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");

    let comments_empty = match inspection.summary.get("comments") {
        None => true,
        Some(value) => value.as_array().is_some_and(|items| items.is_empty()),
    };
    assert!(
        comments_empty,
        "expected omitted or empty comments, got {:?}",
        inspection.summary.get("comments")
    );

    let charts_empty = match inspection.summary.get("charts") {
        None => true,
        Some(value) => value.as_array().is_some_and(|items| items.is_empty()),
    };
    assert!(
        charts_empty,
        "expected omitted or empty charts, got {:?}",
        inspection.summary.get("charts")
    );
}
