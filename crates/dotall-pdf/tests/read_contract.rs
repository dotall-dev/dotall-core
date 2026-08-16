use std::fs;

use dotall_core::registry::{FormatHandler, ReadRequest, ReadSelector};
use dotall_pdf::{PdfFormat, demo_form_pdf, minimal_form_pdf};
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

#[test]
fn demo_form_page_read_returns_labels() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
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
    assert!(
        page.content.contains("Vendor Intake Form"),
        "{}",
        page.content
    );
    assert!(page.content.contains("Name"), "{}", page.content);
    assert!(page.content.contains("Department"), "{}", page.content);
}

#[test]
fn inspect_includes_info_metadata() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    let metadata = &inspection.summary["metadata"];
    assert_eq!(metadata["title"], "Vendor Intake Form");
    assert_eq!(metadata["author"], "Dotall Demo");
    assert_eq!(metadata["subject"], "Vendor onboarding");
    assert_eq!(metadata["creator"], "dotall-pdf");
    assert_eq!(metadata["producer"], "dotall-pdf");
}

#[test]
fn inspect_and_read_expose_choice_options() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("form.pdf");
    fs::write(&path, demo_form_pdf()).expect("write fixture");

    let handler = PdfFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    let fields = inspection.summary["fields"]
        .as_array()
        .expect("fields summary");
    let dept = fields
        .iter()
        .find(|field| field["name"] == "Department")
        .expect("Department field");
    assert_eq!(dept["field_type"], "ch");
    let options = dept["options"]
        .as_array()
        .expect("options")
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(options, vec!["Engineering", "Sales", "Operations"]);

    let field = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "field".into(),
                    value: "Department".into(),
                }),
                max_tokens: 2_000,
                continuation: None,
            },
        )
        .expect("read field");
    assert!(field.content.contains("Engineering"));
    assert!(field.content.contains("Sales"));
    assert!(field.content.contains("Operations"));
}
