use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_docx::{DocxFormat, fixture, parse_document_bytes};
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn set_paragraph_text_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_text".into(),
                payload: serde_json::json!({ "index": 1, "text": "Gamma" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.paragraphs[1].text, "Gamma");
    assert_eq!(after.paragraphs[0].text, "Alpha");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn rejects_tracked_changes() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::tracked_change_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_text".into(),
                payload: serde_json::json!({ "index": 1, "text": "Gamma" }),
            }],
        )
        .expect_err("tracked changes must fail");

    assert!(error.to_string().contains("tracked changes"));
}

#[test]
fn set_paragraph_text_edits_table_cell_and_leaves_header_media_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("table.docx");
    let before = fixture::table_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    assert_eq!(
        model.payload["paragraphs"].as_array().expect("paras").len(),
        3
    );

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_text".into(),
                payload: serde_json::json!({ "index": 1, "text": "UpdatedA" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.paragraphs[0].text, "Intro");
    assert_eq!(after.paragraphs[1].text, "UpdatedA");
    assert_eq!(after.paragraphs[2].text, "CellB");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_text_preserves_first_run_rpr_not_paragraph_mark() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("multi.docx");
    let before = fixture::multi_run_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    assert_eq!(model.payload["paragraphs"][0]["text"], "BoldItalic");

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_text".into(),
                payload: serde_json::json!({ "index": 0, "text": "Hello" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document = document_xml(&patched.bytes);

    assert!(document.contains("<w:t xml:space=\"preserve\">Hello</w:t>"));
    assert!(
        document.contains("<w:r><w:rPr><w:b/></w:rPr>"),
        "must clone first text-run rPr (bold), not paragraph-mark rPr: {document}"
    );
    assert!(
        !document.contains("<w:i/>"),
        "subsequent runs must be cleared: {document}"
    );
    assert!(
        document.contains("<w:pPr><w:rPr><w:sz w:val=\"24\"/></w:rPr></w:pPr>"),
        "paragraph mark rPr must stay in pPr: {document}"
    );
    let after = parse_document_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.paragraphs[0].text, "Hello");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_text_runs_clones_rpr_per_run() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("multi.docx");
    let before = fixture::multi_run_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_text".into(),
                payload: serde_json::json!({
                    "index": 0,
                    "runs": [
                        { "text": "NewBold" },
                        { "text": "NewItalic" }
                    ]
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document = document_xml(&patched.bytes);

    assert!(
        document
            .contains("<w:r><w:rPr><w:b/></w:rPr><w:t xml:space=\"preserve\">NewBold</w:t></w:r>")
    );
    assert!(
        document.contains(
            "<w:r><w:rPr><w:i/></w:rPr><w:t xml:space=\"preserve\">NewItalic</w:t></w:r>"
        )
    );
    assert!(document.contains("<w:pPr><w:rPr><w:sz w:val=\"24\"/></w:rPr></w:pPr>"));
    let after = parse_document_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.paragraphs[0].text, "NewBoldNewItalic");
    assert_eq!(
        edit.semantic_diff[0].after.as_deref(),
        Some("NewBoldNewItalic")
    );
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn demo_memo_multi_run_edit_preserves_bold_and_italic_rpr() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::demo_memo_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_text".into(),
                payload: serde_json::json!({
                    "index": 1,
                    "runs": [
                        { "text": "Pilot complete; " },
                        { "text": "expanding to PPTX and PDF." }
                    ]
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document = document_xml(&patched.bytes);
    assert!(
        document.contains(
            "<w:r><w:rPr><w:b/></w:rPr><w:t xml:space=\"preserve\">Pilot complete; </w:t></w:r>"
        ),
        "bold run lost: {document}"
    );
    assert!(
        document.contains(
            "<w:r><w:rPr><w:i/></w:rPr><w:t xml:space=\"preserve\">expanding to PPTX and PDF.</w:t></w:r>"
        ),
        "italic run lost: {document}"
    );
}

#[test]
fn insert_paragraph_after_body_index_shifts_following() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_paragraph".into(),
                payload: serde_json::json!({ "after": 0, "text": "Inserted" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.paragraphs.len(), 3);
    assert_eq!(after.paragraphs[0].text, "Alpha");
    assert_eq!(after.paragraphs[1].text, "Inserted");
    assert_eq!(after.paragraphs[2].text, "Beta");
    assert_eq!(edit.semantic_diff[0].change, "insert_paragraph");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("Inserted"));
    assert_eq!(edit.semantic_diff[0].target, "paragraph:1");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn insert_paragraph_after_table_cell_inserts_inside_cell() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("table.docx");
    let before = fixture::table_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_paragraph".into(),
                payload: serde_json::json!({ "after": 1, "text": "ExtraInCell" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.paragraphs.len(), 4);
    assert_eq!(after.paragraphs[0].text, "Intro");
    assert_eq!(after.paragraphs[1].text, "CellA");
    assert_eq!(after.paragraphs[2].text, "ExtraInCell");
    assert_eq!(after.paragraphs[3].text, "CellB");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn insert_paragraph_rejects_unknown_after_index() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "insert_paragraph".into(),
                payload: serde_json::json!({ "after": 9, "text": "Nope" }),
            }],
        )
        .expect_err("unknown after index must fail");

    assert!(error.to_string().contains("paragraph `9`"));
}

#[test]
fn delete_paragraph_by_index_shifts_following() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "delete_paragraph".into(),
                payload: serde_json::json!({ "index": 0 }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.paragraphs.len(), 1);
    assert_eq!(after.paragraphs[0].text, "Beta");
    assert_eq!(after.paragraphs[0].index, 0);
    assert_eq!(edit.semantic_diff[0].change, "delete_paragraph");
    assert_eq!(edit.semantic_diff[0].before.as_deref(), Some("Alpha"));
    assert_eq!(edit.semantic_diff[0].after, None);
    assert_eq!(edit.semantic_diff[0].target, "paragraph:0");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn delete_paragraph_by_element_id() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let element_id = model.payload["paragraphs"][1]["element_id"]
        .as_str()
        .expect("element_id")
        .to_owned();
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "delete_paragraph".into(),
                payload: serde_json::json!({ "element_id": element_id }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.paragraphs.len(), 1);
    assert_eq!(after.paragraphs[0].text, "Alpha");
    assert_eq!(edit.operations[0].payload["index"], 1);
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn delete_paragraph_removes_table_cell_paragraph() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("table.docx");
    let before = fixture::table_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "delete_paragraph".into(),
                payload: serde_json::json!({ "index": 1 }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.paragraphs.len(), 2);
    assert_eq!(after.paragraphs[0].text, "Intro");
    assert_eq!(after.paragraphs[1].text, "CellB");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn delete_paragraph_rejects_unknown_index() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "delete_paragraph".into(),
                payload: serde_json::json!({ "index": 9 }),
            }],
        )
        .expect_err("unknown index must fail");

    assert!(error.to_string().contains("paragraph `9`"));
}

#[test]
fn set_paragraph_style_updates_style_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    assert_eq!(
        model.payload["paragraphs"][0]["style_id"].as_str(),
        Some("Heading1")
    );
    assert!(model.payload["paragraphs"][1]["style_id"].is_null());

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_style".into(),
                payload: serde_json::json!({ "index": 1, "style_id": "Heading1" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.paragraphs[1].style_id.as_deref(), Some("Heading1"));
    assert_eq!(after.paragraphs[1].text, "Beta");
    assert_eq!(after.paragraphs[0].style_id.as_deref(), Some("Heading1"));
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_style");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("Heading1"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_alignment_sets_jc_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_alignment".into(),
                payload: serde_json::json!({ "index": 1, "alignment": "center" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains(r#"<w:jc w:val="center"/>"#)
            || document_xml.contains(r#"w:val="center""#),
        "expected w:jc center in document.xml"
    );
    let after = parse_document_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.paragraphs[1].text, "Beta");
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_alignment");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("center"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_alignment_rejects_invalid_value() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_alignment".into(),
                payload: serde_json::json!({ "index": 0, "alignment": "diagonal" }),
            }],
        )
        .expect_err("invalid alignment");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("alignment") || message.contains("left") || message.contains("center"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_paragraph_bold_sets_wb_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_bold".into(),
                payload: serde_json::json!({ "index": 1, "bold": true }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains("<w:b/>")
            || document_xml.contains(r#"<w:b "#)
            || document_xml.contains(r#"w:b w:val="true""#)
            || document_xml.contains(r#"w:b w:val="1""#),
        "expected w:b in document.xml"
    );
    let after = parse_document_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.paragraphs[1].text, "Beta");
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_bold");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("true"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_bold_rejects_non_bool() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_bold".into(),
                payload: serde_json::json!({ "index": 0, "bold": "yes" }),
            }],
        )
        .expect_err("non-bool bold");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("bold") || message.contains("boolean") || message.contains("bool"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_paragraph_italic_sets_wi_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_italic".into(),
                payload: serde_json::json!({ "index": 1, "italic": true }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains("<w:i/>")
            || document_xml.contains(r#"<w:i "#)
            || document_xml.contains(r#"w:i w:val="true""#)
            || document_xml.contains(r#"w:i w:val="1""#),
        "expected w:i in document.xml"
    );
    let after = parse_document_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.paragraphs[1].text, "Beta");
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_italic");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("true"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_italic_rejects_non_bool() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_italic".into(),
                payload: serde_json::json!({ "index": 0, "italic": "yes" }),
            }],
        )
        .expect_err("non-bool italic");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("italic") || message.contains("boolean") || message.contains("bool"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_paragraph_font_size_sets_wsz_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_size".into(),
                payload: serde_json::json!({ "index": 1, "size_pt": 14.0 }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains(r#"w:sz w:val="28""#) || document_xml.contains("w:sz w:val=\"28\""),
        "expected w:sz half-points 28 for 14pt, got: {document_xml}"
    );
    assert!(
        document_xml.contains(r#"w:szCs w:val="28""#)
            || document_xml.contains("w:szCs w:val=\"28\""),
        "expected w:szCs half-points 28 for 14pt"
    );
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_font_size");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("14"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_font_size_clear_removes_sz() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let set = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_size".into(),
                payload: serde_json::json!({ "index": 1, "size_pt": 16.0 }),
            }],
        )
        .expect("validate set");
    let patched = handler.apply_edit(&path, &set).expect("apply set");
    fs::write(&path, &patched.bytes).expect("rewrite");

    let model = handler.parse(&path).expect("parse");
    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_size".into(),
                payload: serde_json::json!({ "index": 1, "size_pt": null }),
            }],
        )
        .expect("validate clear");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let document_xml = String::from_utf8(zip_entries(&cleared.bytes)["word/document.xml"].clone())
        .expect("document xml");
    // Body paragraph index 1 should no longer carry sz; leave other paragraphs alone.
    let para = document_xml.split("<w:p>").nth(2).unwrap_or(&document_xml);
    assert!(
        !para.contains("<w:sz ") && !para.contains("<w:szCs "),
        "cleared font size must remove w:sz/w:szCs from target paragraph"
    );
}

#[test]
fn set_paragraph_font_size_rejects_non_positive() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_size".into(),
                payload: serde_json::json!({ "index": 1, "size_pt": -1 }),
            }],
        )
        .expect_err("non-positive size");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("size") || message.contains("positive"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_paragraph_font_name_sets_rfonts_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_name".into(),
                payload: serde_json::json!({ "index": 1, "font": "Arial" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains(r#"w:ascii="Arial""#) || document_xml.contains("w:ascii=\"Arial\""),
        "expected w:rFonts w:ascii=Arial, got: {document_xml}"
    );
    assert!(
        document_xml.contains("<w:rFonts") || document_xml.contains("w:rFonts"),
        "expected w:rFonts element"
    );
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_font_name");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("Arial"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_font_name_clear_removes_rfonts() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let set = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_name".into(),
                payload: serde_json::json!({ "index": 1, "font": "Calibri" }),
            }],
        )
        .expect("validate set");
    let patched = handler.apply_edit(&path, &set).expect("apply set");
    fs::write(&path, &patched.bytes).expect("rewrite");

    let model = handler.parse(&path).expect("parse");
    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_name".into(),
                payload: serde_json::json!({ "index": 1, "font": null }),
            }],
        )
        .expect("validate clear");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let document_xml = String::from_utf8(zip_entries(&cleared.bytes)["word/document.xml"].clone())
        .expect("document xml");
    let para = document_xml.split("<w:p>").nth(2).unwrap_or(&document_xml);
    assert!(
        !para.contains("<w:rFonts"),
        "cleared font name must remove w:rFonts from target paragraph"
    );
}

#[test]
fn set_paragraph_font_name_rejects_empty() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_name".into(),
                payload: serde_json::json!({ "index": 1, "font": "" }),
            }],
        )
        .expect_err("empty font");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("font") || message.contains("empty") || message.contains("non-empty"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_paragraph_font_color_sets_wcolor_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_color".into(),
                payload: serde_json::json!({ "index": 1, "color": "#C00000" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains(r#"w:val="C00000""#) || document_xml.contains("w:val=\"C00000\""),
        "expected w:color w:val=C00000, got: {document_xml}"
    );
    assert!(
        document_xml.contains("<w:color") || document_xml.contains("w:color"),
        "expected w:color element"
    );
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_font_color");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("C00000"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_font_color_clear_removes_wcolor() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let set = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_color".into(),
                payload: serde_json::json!({ "index": 1, "color": "4472C4" }),
            }],
        )
        .expect("validate set");
    let patched = handler.apply_edit(&path, &set).expect("apply set");
    fs::write(&path, &patched.bytes).expect("rewrite");

    let model = handler.parse(&path).expect("parse");
    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_color".into(),
                payload: serde_json::json!({ "index": 1, "color": null }),
            }],
        )
        .expect("validate clear");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let document_xml = String::from_utf8(zip_entries(&cleared.bytes)["word/document.xml"].clone())
        .expect("document xml");
    let para = document_xml.split("<w:p>").nth(2).unwrap_or(&document_xml);
    assert!(
        !para.contains("<w:color"),
        "cleared font color must remove w:color from target paragraph"
    );
}

#[test]
fn set_paragraph_font_color_rejects_invalid() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_font_color".into(),
                payload: serde_json::json!({ "index": 1, "color": "red" }),
            }],
        )
        .expect_err("invalid color");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("color") || message.contains("hex") || message.contains("rrggbb"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_paragraph_highlight_sets_whighlight_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_highlight".into(),
                payload: serde_json::json!({ "index": 1, "color": "yellow" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains(r#"<w:highlight w:val="yellow"/>"#)
            || document_xml.contains(r#"w:val="yellow""#),
        "expected w:highlight yellow, got: {document_xml}"
    );
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_highlight");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("yellow"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_highlight_clear_removes_whighlight() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let set = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_highlight".into(),
                payload: serde_json::json!({ "index": 1, "color": "cyan" }),
            }],
        )
        .expect("validate set");
    let patched = handler.apply_edit(&path, &set).expect("apply set");
    fs::write(&path, &patched.bytes).expect("rewrite");

    let model = handler.parse(&path).expect("parse");
    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_highlight".into(),
                payload: serde_json::json!({ "index": 1, "color": null }),
            }],
        )
        .expect("validate clear");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let document_xml = String::from_utf8(zip_entries(&cleared.bytes)["word/document.xml"].clone())
        .expect("document xml");
    let para = document_xml.split("<w:p>").nth(2).unwrap_or(&document_xml);
    assert!(
        !para.contains("<w:highlight"),
        "cleared highlight must remove w:highlight from target paragraph"
    );
}

#[test]
fn set_paragraph_highlight_rejects_invalid() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_highlight".into(),
                payload: serde_json::json!({ "index": 1, "color": "neon" }),
            }],
        )
        .expect_err("invalid highlight");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("highlight") || message.contains("color") || message.contains("yellow"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_paragraph_strikethrough_sets_wstrike_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_strikethrough".into(),
                payload: serde_json::json!({ "index": 1, "strikethrough": true }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains("<w:strike/>") || document_xml.contains("<w:strike "),
        "expected w:strike in document.xml: {document_xml}"
    );
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_strikethrough");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("true"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_strikethrough_clear_sets_val_zero() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let set = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_strikethrough".into(),
                payload: serde_json::json!({ "index": 1, "strikethrough": true }),
            }],
        )
        .expect("validate set");
    let patched = handler.apply_edit(&path, &set).expect("apply set");
    fs::write(&path, &patched.bytes).expect("rewrite");

    let model = handler.parse(&path).expect("parse");
    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_strikethrough".into(),
                payload: serde_json::json!({ "index": 1, "strikethrough": false }),
            }],
        )
        .expect("validate clear");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let document_xml = String::from_utf8(zip_entries(&cleared.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains(r#"<w:strike w:val="0"/>"#) || document_xml.contains(r#"w:val="0""#),
        "expected w:strike w:val=0 when cleared: {document_xml}"
    );
}

#[test]
fn capabilities_advertise_set_paragraph_strikethrough() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");
    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "set_paragraph_strikethrough"),
        "capabilities must advertise set_paragraph_strikethrough"
    );
}

#[test]
fn set_paragraph_vert_align_superscript_sets_wvert_align_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_vert_align".into(),
                payload: serde_json::json!({ "index": 1, "vert_align": "superscript" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains(r#"<w:vertAlign w:val="superscript"/>"#),
        "expected w:vertAlign superscript in document.xml: {document_xml}"
    );
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_vert_align");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("superscript"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_vert_align_subscript_and_clear() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let set = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_vert_align".into(),
                payload: serde_json::json!({ "index": 1, "vert_align": "subscript" }),
            }],
        )
        .expect("validate set");
    let patched = handler.apply_edit(&path, &set).expect("apply set");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains(r#"<w:vertAlign w:val="subscript"/>"#),
        "expected w:vertAlign subscript: {document_xml}"
    );
    fs::write(&path, &patched.bytes).expect("rewrite");

    let model = handler.parse(&path).expect("parse");
    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_vert_align".into(),
                payload: serde_json::json!({ "index": 1, "vert_align": null }),
            }],
        )
        .expect("validate clear");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let document_xml = String::from_utf8(zip_entries(&cleared.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        !document_xml.contains("w:vertAlign"),
        "expected w:vertAlign cleared: {document_xml}"
    );
}

#[test]
fn set_paragraph_vert_align_rejects_invalid_value() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_vert_align".into(),
                payload: serde_json::json!({ "index": 1, "vert_align": "baseline" }),
            }],
        )
        .expect_err("invalid vert_align");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("vert_align")
            || message.contains("superscript")
            || message.contains("subscript"),
        "unexpected error: {error}"
    );
}

#[test]
fn capabilities_advertise_set_paragraph_vert_align() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");
    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "set_paragraph_vert_align"),
        "capabilities must advertise set_paragraph_vert_align"
    );
}

#[test]
fn set_paragraph_caps_small_sets_wsmall_caps_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_caps".into(),
                payload: serde_json::json!({ "index": 1, "caps": "small" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains("<w:smallCaps") || document_xml.contains("<w:smallCaps/>"),
        "expected w:smallCaps in document.xml: {document_xml}"
    );
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_caps");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("small"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_caps_all_and_clear() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let set = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_caps".into(),
                payload: serde_json::json!({ "index": 1, "caps": "all" }),
            }],
        )
        .expect("validate set");
    let patched = handler.apply_edit(&path, &set).expect("apply set");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains("<w:caps") || document_xml.contains("<w:caps/>"),
        "expected w:caps: {document_xml}"
    );
    fs::write(&path, &patched.bytes).expect("rewrite");

    let model = handler.parse(&path).expect("parse");
    let clear = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_caps".into(),
                payload: serde_json::json!({ "index": 1, "caps": null }),
            }],
        )
        .expect("validate clear");
    let cleared = handler.apply_edit(&path, &clear).expect("apply clear");
    let document_xml = String::from_utf8(zip_entries(&cleared.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        !document_xml.contains("w:smallCaps") && !document_xml.contains("<w:caps"),
        "expected caps cleared: {document_xml}"
    );
}

#[test]
fn set_paragraph_caps_rejects_invalid_value() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_caps".into(),
                payload: serde_json::json!({ "index": 1, "caps": "title" }),
            }],
        )
        .expect_err("invalid caps");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("caps") || message.contains("small") || message.contains("all"),
        "unexpected error: {error}"
    );
}

#[test]
fn capabilities_advertise_set_paragraph_caps() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");
    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "set_paragraph_caps"),
        "capabilities must advertise set_paragraph_caps"
    );
}

#[test]
fn set_paragraph_underline_sets_wu_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_underline".into(),
                payload: serde_json::json!({ "index": 1, "underline": true }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let document_xml = String::from_utf8(zip_entries(&patched.bytes)["word/document.xml"].clone())
        .expect("document xml");
    assert!(
        document_xml.contains(r#"<w:u w:val="single"/>"#)
            || document_xml.contains(r#"w:val="single""#),
        "expected w:u single in document.xml: {document_xml}"
    );
    let after = parse_document_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.paragraphs[1].text, "Beta");
    assert_eq!(edit.semantic_diff[0].change, "set_paragraph_underline");
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("true"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn set_paragraph_underline_rejects_non_bool() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let error = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_underline".into(),
                payload: serde_json::json!({ "index": 0, "underline": "yes" }),
            }],
        )
        .expect_err("non-bool underline");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("underline") || message.contains("boolean") || message.contains("bool"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_paragraph_style_replaces_existing_pstyle() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_paragraph_style".into(),
                payload: serde_json::json!({ "index": 0, "style_id": "Title" }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");
    assert_eq!(after.paragraphs[0].style_id.as_deref(), Some("Title"));
    assert_eq!(edit.semantic_diff[0].before.as_deref(), Some("Heading1"));
}

#[test]
fn set_header_paragraph_text_patches_only_header_part() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("table.docx");
    let before = fixture::table_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    assert_eq!(
        model.payload["header_paragraphs"]
            .as_array()
            .expect("headers")
            .len(),
        1
    );

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_header_paragraph_text".into(),
                payload: serde_json::json!({
                    "part": "header1",
                    "index": 0,
                    "text": "CONFIDENTIAL"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.header_paragraphs[0].text, "CONFIDENTIAL");
    assert_eq!(after.paragraphs[0].text, "Intro");
    assert_eq!(after.paragraphs[1].text, "CellA");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/header1.xml"]);
}

#[test]
fn set_footer_paragraph_text_patches_only_footer_part() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("footer.docx");
    let before = fixture::header_footer_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    assert_eq!(after_footer_count(&model), 1);

    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_footer_paragraph_text".into(),
                payload: serde_json::json!({
                    "part": "footer1",
                    "index": 0,
                    "text": "Page footer updated"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.footer_paragraphs[0].text, "Page footer updated");
    assert_eq!(after.header_paragraphs[0].text, "HeaderOnly");
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/footer1.xml"]);
}

fn after_footer_count(model: &dotall_core::registry::ArtifactEnvelope) -> usize {
    model.payload["footer_paragraphs"]
        .as_array()
        .expect("footers")
        .len()
}

fn document_xml(package: &[u8]) -> String {
    let entries = zip_entries(package);
    let bytes = entries.get("word/document.xml").expect("document.xml");
    String::from_utf8(bytes.clone()).expect("utf-8 document")
}

fn assert_untouched_entries_identical(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);
    assert_eq!(before_entries.len(), after_entries.len());
    for (name, before_bytes) in &before_entries {
        if patched.contains(&name.as_str()) {
            continue;
        }
        let after_bytes = after_entries.get(name).expect("entry retained");
        assert_eq!(before_bytes, after_bytes, "bytes changed for {name}");
    }
}

fn zip_entries(package: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package)).expect("zip");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("entry");
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read");
        entries.insert(name, bytes);
    }
    entries
}

#[test]
fn replace_paragraph_text_find_replace_and_leaves_other_parts_byte_identical() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    let before = fixture::minimal_docx();
    fs::write(&path, &before).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "replace_paragraph_text".into(),
                payload: serde_json::json!({
                    "index": 1,
                    "find": "eta",
                    "replace": "ETA",
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&path, &edit).expect("apply");
    let after = parse_document_bytes(&patched.bytes).expect("reparse");

    assert_eq!(after.paragraphs[1].text, "BETA");
    assert_eq!(edit.semantic_diff[0].change, "replace_paragraph_text");
    assert_eq!(edit.semantic_diff[0].before.as_deref(), Some("Beta"));
    assert_eq!(edit.semantic_diff[0].after.as_deref(), Some("BETA"));
    assert_untouched_entries_identical(&before, &patched.bytes, &["word/document.xml"]);
}

#[test]
fn replace_paragraph_text_rejects_empty_find_and_no_match() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");

    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let empty = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "replace_paragraph_text".into(),
                payload: serde_json::json!({
                    "index": 1,
                    "find": "",
                    "replace": "x",
                }),
            }],
        )
        .expect_err("empty find");
    assert!(
        empty.to_string().to_lowercase().contains("find"),
        "unexpected: {empty}"
    );

    let missing = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "replace_paragraph_text".into(),
                payload: serde_json::json!({
                    "index": 1,
                    "find": "zzz",
                    "replace": "x",
                }),
            }],
        )
        .expect_err("no match");
    let message = missing.to_string().to_lowercase();
    assert!(
        message.contains("find") || message.contains("not found") || message.contains("match"),
        "unexpected: {missing}"
    );
}

#[test]
fn capabilities_advertise_replace_paragraph_text() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("memo.docx");
    fs::write(&path, fixture::minimal_docx()).expect("write fixture");
    let handler = DocxFormat;
    let model = handler.parse(&path).expect("parse");
    let inspection = handler.inspect(&model).expect("inspect");
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "replace_paragraph_text"),
        "capabilities must advertise replace_paragraph_text"
    );
}
