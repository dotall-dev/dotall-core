use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::{XlsxFormat, parse_workbook};
use rust_xlsxwriter::{Chart, ChartType, Note, Workbook};
use tempfile::tempdir;
use zip::ZipArchive;

#[test]
fn inspect_exposes_legacy_comments() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("with_comment.xlsx");
    write_fixture_with_comment(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let workbook = parse_workbook(&source).expect("parse workbook");
    assert!(
        !workbook.comments.is_empty(),
        "parsed model must include comments"
    );
    let comment = &workbook.comments[0];
    assert_eq!(comment.sheet, "Inputs");
    assert_eq!(comment.cell, "B2");
    assert_eq!(comment.author, "Ada");
    assert_eq!(comment.text, "Check Rate");
    assert!(
        comment.element_id.starts_with("cm_"),
        "comment element_id must use cm_ prefix: {}",
        comment.element_id
    );

    let inspection = handler.inspect(&model).expect("inspect");
    let comments = inspection.summary["comments"]
        .as_array()
        .expect("summary.comments array");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["sheet"], "Inputs");
    assert_eq!(comments[0]["cell"], "B2");
    assert_eq!(comments[0]["author"], "Ada");
    assert_eq!(comments[0]["text"], "Check Rate");
    assert!(
        comments[0]["element_id"]
            .as_str()
            .expect("element_id")
            .starts_with("cm_"),
        "inspect comment element_id must use cm_ prefix"
    );
    assert!(
        inspection
            .edit_capabilities
            .iter()
            .any(|cap| cap.operation == "insert_comment"),
        "capabilities must advertise insert_comment"
    );
}

#[test]
fn inspect_returns_empty_comments_when_absent() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("no_comment.xlsx");
    write_fixture_plain(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let workbook = parse_workbook(&source).expect("parse workbook");
    assert!(workbook.comments.is_empty());

    let inspection = handler.inspect(&model).expect("inspect");
    let comments = inspection.summary.get("comments");
    assert!(
        comments.is_none()
            || comments
                .and_then(|value| value.as_array())
                .is_some_and(|array| array.is_empty()),
        "summary.comments must be omitted or [] when absent, got {comments:?}"
    );
}

#[test]
fn inspect_exposes_charts_with_title() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("with_chart.xlsx");
    write_fixture_with_chart(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let workbook = parse_workbook(&source).expect("parse workbook");
    assert!(
        !workbook.charts.is_empty(),
        "parsed model must include charts"
    );
    let chart = &workbook.charts[0];
    assert_eq!(chart.title, "Revenue");
    assert_eq!(chart.sheet.as_deref(), Some("Revenue"));
    assert!(
        chart.element_id.starts_with("ch_"),
        "chart element_id must use ch_ prefix: {}",
        chart.element_id
    );

    let inspection = handler.inspect(&model).expect("inspect");
    let charts = inspection.summary["charts"]
        .as_array()
        .expect("summary.charts array");
    assert_eq!(charts.len(), 1);
    assert_eq!(charts[0]["title"], "Revenue");
    assert_eq!(charts[0]["sheet"], "Revenue");
    assert!(
        charts[0]["element_id"]
            .as_str()
            .expect("element_id")
            .starts_with("ch_"),
        "inspect chart element_id must use ch_ prefix"
    );
    assert_eq!(
        inspection.summary["preserved"][0], "charts",
        "inspect list must not remove preserved charts entry"
    );
}

#[test]
fn insert_comment_adds_legacy_note_and_reinspects() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("insert_comment.xlsx");
    write_fixture_plain(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B2",
                    "text": "Review Rate",
                    "author": "Dotall"
                }),
            }],
        )
        .expect("validate insert_comment");
    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply insert_comment");

    let entries = zip_entries(&patched.bytes);
    assert!(
        entries.keys().any(|name| name.starts_with("xl/comments")),
        "expected comments*.xml part"
    );
    assert!(
        entries
            .keys()
            .any(|name| name.contains("vmlDrawing") && name.ends_with(".vml")),
        "expected vmlDrawing*.vml part"
    );
    let comments_xml = entries
        .iter()
        .find(|(name, _)| name.starts_with("xl/comments"))
        .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
        .expect("comments xml");
    assert!(
        comments_xml.contains(r#"ref="B2""#) && comments_xml.contains("Review Rate"),
        "comments XML must contain cell + text: {comments_xml}"
    );

    assert_entry_byte_identical(&before, &patched.bytes, "xl/worksheets/sheet2.xml");
    assert_untouched_entries_are_identical(
        &before,
        &patched.bytes,
        &[
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/_rels/sheet1.xml.rels",
            "[Content_Types].xml",
        ],
    );

    let after_path = directory.path().join("patched.xlsx");
    fs::write(&after_path, &patched.bytes).expect("write patched");
    let after_model = handler.parse(&after_path).expect("re-parse");
    let inspection = handler.inspect(&after_model).expect("re-inspect");
    let comments = inspection.summary["comments"]
        .as_array()
        .expect("summary.comments");
    let inserted = comments
        .iter()
        .find(|comment| comment["cell"] == "B2" && comment["sheet"] == "Inputs")
        .expect("inserted comment");
    assert_eq!(inserted["text"], "Review Rate");
    assert_eq!(inserted["author"], "Dotall");
}

#[test]
fn insert_comment_defaults_author_to_dotall() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("default_author.xlsx");
    write_fixture_plain(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let edit = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B2",
                    "text": "Review Rate"
                }),
            }],
        )
        .expect("validate insert_comment without author");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let after_path = directory.path().join("patched.xlsx");
    fs::write(&after_path, patched.bytes).expect("write");
    let after = parse_workbook(&after_path).expect("parse");
    let comment = after
        .comments
        .iter()
        .find(|comment| comment.cell == "B2")
        .expect("comment");
    assert_eq!(comment.author, "Dotall");
}

#[test]
fn insert_comment_rejects_when_cell_already_has_comment() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("existing_comment.xlsx");
    write_fixture_with_comment(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let error = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "insert_comment".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B2",
                    "text": "Duplicate",
                    "author": "Dotall"
                }),
            }],
        )
        .expect_err("existing comment must reject insert");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("comment") || message.contains("already"),
        "expected existing-comment rejection, got {error}"
    );
}

#[test]
fn rejects_comment_mutate_kinds() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("comment_mutate.xlsx");
    write_fixture_with_comment(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    for kind in ["set_comment", "delete_comment", "replace_comment"] {
        let error = handler
            .validate_edit_with_source(
                &source,
                &model,
                &[SemanticOperation {
                    kind: kind.into(),
                    payload: serde_json::json!({
                        "sheet": "Inputs",
                        "address": "B2",
                        "text": "Nope"
                    }),
                }],
            )
            .expect_err("comment mutate must reject");
        let message = error.to_string();
        assert!(
            message.contains("UnsupportedCapability")
                || message.to_lowercase().contains("unsupported")
                || message.contains("insert_comment"),
            "expected UnsupportedCapability/format listing available ops for `{kind}`, got {error}"
        );
    }
}

#[test]
fn rejects_chart_mutate_kind() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("chart_mutate.xlsx");
    write_fixture_with_chart(&source);

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse");
    let error = handler
        .validate_edit_with_source(
            &source,
            &model,
            &[SemanticOperation {
                kind: "set_chart_title".into(),
                payload: serde_json::json!({
                    "sheet": "Revenue",
                    "title": "Mutated"
                }),
            }],
        )
        .expect_err("chart mutate must reject");
    let message = error.to_string();
    assert!(
        message.contains("UnsupportedCapability")
            || message.to_lowercase().contains("unsupported")
            || message.to_lowercase().contains("chart"),
        "expected chart mutate rejection, got {error}"
    );
}

fn write_fixture_plain(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("name");
    inputs.write(1, 0, "Rate").expect("label");
    inputs.write(1, 1, 0.1).expect("rate");
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("name");
    revenue.write(0, 0, "Total").expect("label");
    revenue.write(0, 1, 10.0).expect("value");
    workbook.save(path).expect("save");
}

fn write_fixture_with_comment(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("name");
    inputs.set_default_note_author("Ada");
    inputs.write(1, 0, "Rate").expect("label");
    inputs.write(1, 1, 0.1).expect("rate");
    let note = Note::new("Check Rate").add_author_prefix(false);
    inputs.insert_note(1, 1, &note).expect("insert note");
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("name");
    revenue.write(0, 0, "Total").expect("label");
    revenue.write(0, 1, 10.0).expect("value");
    workbook.save(path).expect("save");
}

fn write_fixture_with_chart(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("name");
    inputs.write(1, 0, "Rate").expect("label");
    inputs.write(1, 1, 0.1).expect("rate");
    let revenue = workbook.add_worksheet().set_name("Revenue").expect("name");
    revenue.write(0, 0, "Total").expect("label");
    revenue.write(0, 1, 10.0).expect("v1");
    revenue.write(1, 1, 20.0).expect("v2");
    let mut chart = Chart::new(ChartType::Column);
    chart.title().set_name("Revenue");
    chart.add_series().set_values(("Revenue", 0, 1, 1, 1));
    revenue.insert_chart(3, 0, &chart).expect("chart");
    workbook.save(path).expect("save");
}

fn assert_entry_byte_identical(before: &[u8], after: &[u8], name: &str) {
    assert_eq!(
        zip_entries(before).get(name),
        zip_entries(after).get(name),
        "entry `{name}` must stay byte-identical"
    );
}

fn assert_untouched_entries_are_identical(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);
    for (name, before_bytes) in &before_entries {
        if patched.contains(&name.as_str()) {
            continue;
        }
        // New comment parts may appear; existing entries not listed as patched must match.
        let after_bytes = after_entries
            .get(name)
            .unwrap_or_else(|| panic!("entry `{name}` must be retained"));
        assert_eq!(before_bytes, after_bytes, "untouched part `{name}` changed");
    }
}

fn zip_entries(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open ZIP");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("ZIP entry");
        let name = entry.name().to_owned();
        let mut inflated = Vec::new();
        entry.read_to_end(&mut inflated).expect("read entry");
        entries.insert(name, inflated);
    }
    entries
}
