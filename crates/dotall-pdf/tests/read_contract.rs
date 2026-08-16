use std::fs;

use dotall_core::registry::{FormatHandler, ReadRequest, ReadSelector};
use dotall_pdf::{PdfFormat, minimal_form_pdf};
use tempfile::tempdir;

#[test]
fn inspect_and_read_page_and_field() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, minimal_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert_eq!(inspection.format_id, "pdf");
    assert_eq!(inspection.summary["page_count"], 1);
    assert_eq!(inspection.summary["field_names"][0], "Name");

    let page = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "page".into(),
                    value: "1".into(),
                }),
                max_tokens: 2_000,
                continuation: None,
            },
        )
        .expect("read page");
    assert!(page.content.contains("Hello"));

    let field = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "field".into(),
                    value: "Name".into(),
                }),
                max_tokens: 2_000,
                continuation: None,
            },
        )
        .expect("read field");
    assert!(field.content.contains("Ada"));
}
