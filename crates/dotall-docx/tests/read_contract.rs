use std::fs;

use dotall_core::registry::{FormatHandler, ReadRequest, ReadSelector};
use dotall_docx::{DocxFormat, minimal_docx};
use tempfile::tempdir;

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
