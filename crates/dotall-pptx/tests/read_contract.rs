use std::fs;

use dotall_core::registry::{FormatHandler, ReadRequest, ReadSelector};
use dotall_pptx::{PptxFormat, minimal_pptx, minimal_pptx_with_media, pptx_with_table};
use tempfile::tempdir;

#[test]
fn inspect_suggests_slide_reads_and_lists_media() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx_with_media(true)).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");

    assert_eq!(inspection.format_id, "pptx");
    assert_eq!(inspection.summary["slides"][0]["name"], "Slide 1");
    assert_eq!(inspection.summary["slides"][0]["preview"], "Hello");
    assert_eq!(inspection.summary["media_parts"][0], "ppt/media/logo.bin");
    assert!(
        inspection
            .suggested_reads
            .iter()
            .any(|suggestion| suggestion.selector.kind == "slide")
    );
}

#[test]
fn read_slide_returns_shape_text() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let response = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "slide".into(),
                    value: "Slide 1".into(),
                }),
                max_tokens: 2_000,
                continuation: None,
            },
        )
        .expect("read slide");

    assert!(response.content.contains("Hello"));
    assert!(response.content.contains("Title"));
    assert!(!response.truncated);
}

#[test]
fn tiny_token_budget_sets_continuation() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, minimal_pptx()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let first = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "slide".into(),
                    value: "1".into(),
                }),
                max_tokens: 1,
                continuation: None,
            },
        )
        .expect("first slice");

    assert!(first.truncated);
    let continuation = first.continuation.expect("continuation cursor");
    let second = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "slide".into(),
                    value: "1".into(),
                }),
                max_tokens: 2_000,
                continuation: Some(continuation),
            },
        )
        .expect("resume");

    assert!(!format!("{}{}", first.content, second.content).is_empty());
}

#[test]
fn read_slide_exposes_table_cell_texts() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("deck.pptx");
    fs::write(&path, pptx_with_table()).expect("write fixture");

    let handler = PptxFormat;
    let model = handler.parse(&path).expect("parse");
    let presentation: dotall_pptx::PresentationModel =
        serde_json::from_value(model.payload.clone()).expect("decode");
    let slide = &presentation.slides[0];
    assert_eq!(slide.tables.len(), 1);
    assert_eq!(slide.tables[0].name, "Table 1");
    let texts: Vec<_> = slide.tables[0]
        .cells
        .iter()
        .map(|cell| (cell.row, cell.col, cell.text.as_str()))
        .collect();
    assert_eq!(
        texts,
        vec![(0, 0, "A1"), (0, 1, "B1"), (1, 0, "A2"), (1, 1, "B2")]
    );

    let response = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "slide".into(),
                    value: "Slide 1".into(),
                }),
                max_tokens: 2_000,
                continuation: None,
            },
        )
        .expect("read slide");
    assert!(response.content.contains("A1"));
    assert!(response.content.contains("B2"));
    assert!(response.content.contains("Table 1"));
}
