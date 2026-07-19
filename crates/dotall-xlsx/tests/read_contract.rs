use dotall_core::DotallError;
use dotall_core::registry::{DetectionProbe, FormatHandler, ReadRequest, ReadSelector};
use dotall_xlsx::{CellValue, XlsxFormat, parse_workbook};
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

#[test]
fn parses_sparse_cells_formulas_and_deterministic_hybrid_ids() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("financials.xlsx");

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Revenue").expect("sheet name");
    worksheet.write_string(0, 0, "Month").expect("header");
    worksheet.write_number(1, 0, 100.0).expect("value");
    worksheet.write_formula(1, 1, "=A2*1.1").expect("formula");
    workbook.save(&path).expect("workbook fixture");

    let parsed = parse_workbook(&path).expect("parse workbook");
    let reparsed = parse_workbook(&path).expect("reparse workbook");

    assert_eq!(parsed.workbook_id, reparsed.workbook_id);
    assert_eq!(parsed.sheets.len(), 1);

    let sheet = &parsed.sheets[0];
    assert_eq!(sheet.name, "Revenue");
    assert_eq!(sheet.cells.len(), 3, "empty cells remain sparse");

    let formula_cell = sheet
        .cells
        .iter()
        .find(|cell| cell.address == "B2")
        .expect("formula cell");
    assert_eq!(formula_cell.row, 2);
    assert_eq!(formula_cell.col, 2);
    assert_eq!(formula_cell.formula.as_deref(), Some("=A2*1.1"));
    assert!(formula_cell.element_id.starts_with("c_"));
    assert_ne!(formula_cell.element_id, formula_cell.address);
    assert_eq!(formula_cell.value, CellValue::Float(0.0));

    let reparsed_formula_cell = reparsed.sheets[0]
        .cells
        .iter()
        .find(|cell| cell.address == "B2")
        .expect("reparsed formula cell");
    assert_eq!(formula_cell.element_id, reparsed_formula_cell.element_id);
}

#[test]
fn dimensions_cover_independent_farthest_row_and_column() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("dimensions.xlsx");

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet
        .write_string(0, 25, "far column")
        .expect("far column");
    worksheet.write_string(9, 0, "far row").expect("far row");
    workbook.save(&path).expect("workbook fixture");

    let parsed = parse_workbook(&path).expect("parse workbook");

    assert_eq!(parsed.sheets[0].dimensions.rows, 10);
    assert_eq!(parsed.sheets[0].dimensions.cols, 26);
}

fn workbook_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Revenue").expect("sheet name");
    worksheet.write_string(0, 0, "Month").expect("header");
    worksheet.write_string(0, 1, "Amount").expect("header");
    worksheet.write_string(1, 0, "January").expect("value");
    worksheet.write_number(1, 1, 100.0).expect("value");
    worksheet.write_formula(2, 1, "=B2*1.1").expect("formula");
    workbook.save(path).expect("workbook fixture");
}

#[test]
fn format_handler_detects_xlsx_and_inspects_structure() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("financials.xlsx");
    workbook_fixture(&path);

    let handler = XlsxFormat;
    let score = handler.detect(&DetectionProbe {
        path: &path,
        prefix: b"PK\x03\x04",
    });
    let model = handler.parse(&path).expect("parse through handler");
    let inspection = handler.inspect(&model).expect("inspect workbook");

    assert!(score.0 >= 90);
    assert_eq!(inspection.format_id, "xlsx");
    assert_eq!(inspection.summary["sheets"][0]["rows"], 3);
    assert_eq!(inspection.summary["sheets"][0]["cols"], 2);
    assert_eq!(inspection.summary["sheets"][0]["formula_count"], 1);
    assert_eq!(inspection.summary["preserved"][0], "charts");
    assert_eq!(
        inspection.summary["structure"]["sheets"][0]["header_row"],
        1
    );
    assert_eq!(inspection.suggested_reads[0].selector.kind, "range");
    assert_eq!(inspection.suggested_reads[1].selector.kind, "ast_range");
}

#[test]
fn format_handler_projects_markdown_range_with_drill_down_hint() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("financials.xlsx");
    workbook_fixture(&path);

    let handler = XlsxFormat;
    let model = handler.parse(&path).expect("parse through handler");
    let response = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "range".into(),
                    value: "Revenue!A1:B2".into(),
                }),
                max_tokens: 1_000,
                continuation: None,
            },
        )
        .expect("read range");

    assert!(response.content.contains("| Month | Amount |"));
    assert!(response.content.contains("| January | 100 |"));
    assert!(
        response
            .next_actions
            .iter()
            .any(|action| action.contains("ast_range"))
    );
}

#[test]
fn format_handler_projects_ast_range_as_hybrid_id_json() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("financials.xlsx");
    workbook_fixture(&path);

    let handler = XlsxFormat;
    let model = handler.parse(&path).expect("parse through handler");
    let response = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "ast_range".into(),
                    value: "Revenue!A1:B2".into(),
                }),
                max_tokens: 1_000,
                continuation: None,
            },
        )
        .expect("read AST range");
    let slice: serde_json::Value =
        serde_json::from_str(&response.content).expect("structured JSON");

    assert_eq!(slice["sheet"]["name"], "Revenue");
    assert_eq!(slice["cells"][0]["address"], "A1");
    assert!(
        slice["cells"][0]["element_id"]
            .as_str()
            .expect("element ID")
            .starts_with("c_")
    );
}

#[test]
fn format_handler_rejects_mismatched_artifact_schema() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("financials.xlsx");
    workbook_fixture(&path);

    let handler = XlsxFormat;
    let mut model = handler.parse(&path).expect("parse through handler");
    model.schema_version = 2;

    assert!(matches!(
        handler.inspect(&model),
        Err(DotallError::ArtifactSchemaMismatch {
            schema_id,
            schema_version: 2,
            ..
        }) if schema_id == "xlsx.workbook"
    ));
}

#[test]
fn format_handler_reports_available_reads_for_unknown_selector() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("financials.xlsx");
    workbook_fixture(&path);

    let handler = XlsxFormat;
    let model = handler.parse(&path).expect("parse through handler");
    let error = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "cells".into(),
                    value: "Revenue!A1".into(),
                }),
                max_tokens: 1_000,
                continuation: None,
            },
        )
        .expect_err("unknown selector");

    assert!(matches!(
        error,
        DotallError::UnsupportedCapability { available, .. }
            if available.iter().any(|capability| capability == "read.ast_range")
    ));
}

#[test]
fn format_handler_reports_missing_sheet_as_format_error() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("financials.xlsx");
    workbook_fixture(&path);

    let handler = XlsxFormat;
    let model = handler.parse(&path).expect("parse through handler");
    let error = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "sheet".into(),
                    value: "Expenses".into(),
                }),
                max_tokens: 1_000,
                continuation: None,
            },
        )
        .expect_err("missing sheet");

    assert!(matches!(
        error,
        DotallError::Format { message, .. } if message == "sheet not found: Expenses"
    ));
}

#[test]
fn format_handler_reports_bad_range_syntax_as_format_error() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("financials.xlsx");
    workbook_fixture(&path);

    let handler = XlsxFormat;
    let model = handler.parse(&path).expect("parse through handler");
    let error = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "range".into(),
                    value: "Revenue!A1:invalid".into(),
                }),
                max_tokens: 1_000,
                continuation: None,
            },
        )
        .expect_err("invalid range syntax");

    assert!(matches!(
        error,
        DotallError::Format { message, .. } if message == "invalid cell address `invalid`"
    ));
}
