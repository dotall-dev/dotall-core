use dotall_core::DotallError;
use dotall_core::registry::{DetectionProbe, FormatHandler, ReadRequest, ReadSelector};
use dotall_xlsx::{
    CellModel, CellValue, PreservationStatus, SCHEMA_ID, SCHEMA_VERSION, SheetDimensions,
    SheetModel, UnmodeledMap, WorkbookModel, XlsxFormat, parse_workbook,
};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};
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
        inspection
            .edit_capabilities
            .iter()
            .map(|capability| capability.operation.as_str())
            .collect::<Vec<_>>(),
        vec![
            "set_cell_value",
            "set_cell_formula",
            "set_range",
            "insert_row",
            "delete_row",
            "insert_column",
            "delete_column",
            "add_sheet",
            "rename_sheet",
            "delete_sheet",
            "merge_cells",
            "unmerge_cells",
            "set_column_width",
            "set_row_height",
        ]
    );
    assert_eq!(
        inspection.summary["structure"]["sheets"][0]["header_row"],
        1
    );
    assert_eq!(inspection.suggested_reads[0].selector.kind, "range");
    assert_eq!(inspection.suggested_reads[1].selector.kind, "ast_range");
}

#[test]
fn format_handler_inspects_merges_and_named_range_formulas() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("merges_and_names.xlsx");

    let mut workbook = Workbook::new();
    let inputs = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    inputs
        .merge_range(0, 0, 1, 1, "title", &Format::new())
        .expect("merge A1:B2");
    workbook
        .define_name("Rate", "=Inputs!$B$2")
        .expect("defined name");
    workbook.save(&path).expect("workbook fixture");

    let handler = XlsxFormat;
    let model = handler.parse(&path).expect("parse through handler");
    let inspection = handler.inspect(&model).expect("inspect workbook");

    let sheets = inspection.summary["sheets"]
        .as_array()
        .expect("sheets array");
    let inputs_sheet = sheets
        .iter()
        .find(|sheet| sheet["name"] == "Inputs")
        .expect("Inputs sheet summary");
    assert_eq!(inputs_sheet["merges"][0], "A1:B2");

    let named_ranges = inspection.summary["named_ranges"]
        .as_array()
        .expect("named_ranges array");
    let rate = named_ranges
        .iter()
        .find(|range| range["name"] == "Rate")
        .expect("Rate named range");
    let formula = rate["formula"].as_str().expect("Rate formula");
    assert!(
        formula.contains("Inputs!$B$2"),
        "expected named range formula to reference Inputs!$B$2, got {formula}"
    );

    let response = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "named_ranges".into(),
                    value: String::new(),
                }),
                max_tokens: 1_000,
                continuation: None,
            },
        )
        .expect("read named_ranges");
    let named: serde_json::Value =
        serde_json::from_str(&response.content).expect("named_ranges JSON");
    assert_eq!(named[0]["name"], "Rate");
    assert!(
        named[0]["formula"]
            .as_str()
            .expect("formula")
            .contains("Inputs!$B$2")
    );

    let merges_response = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "merges".into(),
                    value: "Inputs".into(),
                }),
                max_tokens: 1_000,
                continuation: None,
            },
        )
        .expect("read merges");
    let merges: serde_json::Value =
        serde_json::from_str(&merges_response.content).expect("merges JSON");
    assert_eq!(merges["sheet"], "Inputs");
    assert_eq!(merges["merges"][0], "A1:B2");
}

#[test]
fn format_handler_surfaces_cell_number_format_and_style_id() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("styled.xlsx");

    let percent = Format::new().set_num_format("0%");
    let currency = Format::new().set_num_format("$#,##0.00");

    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    sheet
        .write_number_with_format(0, 0, 0.10, &percent)
        .expect("percent cell");
    sheet
        .write_number_with_format(1, 0, 100.0, &currency)
        .expect("currency cell");
    workbook.save(&path).expect("workbook fixture");

    let handler = XlsxFormat;
    let model = handler.parse(&path).expect("parse through handler");
    let workbook_model = parse_workbook(&path).expect("parse workbook model");

    let percent_cell = workbook_model.sheets[0]
        .cells
        .iter()
        .find(|cell| cell.address == "A1")
        .expect("percent cell");
    assert_eq!(percent_cell.number_format.as_deref(), Some("0%"));
    let percent_style = percent_cell.style_id.as_deref().expect("percent style_id");
    assert!(
        percent_style.starts_with("st_"),
        "expected style_id prefix st_, got {percent_style}"
    );

    let currency_cell = workbook_model.sheets[0]
        .cells
        .iter()
        .find(|cell| cell.address == "A2")
        .expect("currency cell");
    assert_eq!(currency_cell.number_format.as_deref(), Some("$#,##0.00"));
    assert!(
        currency_cell
            .style_id
            .as_ref()
            .is_some_and(|id| id.starts_with("st_"))
    );

    assert!(
        !workbook_model.style_table.is_empty(),
        "style_table should list style entries used by the workbook"
    );
    assert!(
        workbook_model
            .style_table
            .iter()
            .any(|entry| entry.style_id == percent_style)
    );

    let inspection = handler.inspect(&model).expect("inspect workbook");
    let style_table = inspection.summary["style_table"]
        .as_array()
        .expect("inspect style_table");
    assert!(
        !style_table.is_empty(),
        "inspect summary should surface style_table ids"
    );

    let response = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "ast_range".into(),
                    value: "Inputs!A1:A2".into(),
                }),
                max_tokens: 1_000,
                continuation: None,
            },
        )
        .expect("read ast_range");
    let slice: serde_json::Value = serde_json::from_str(&response.content).expect("ast_range JSON");
    let cells = slice["cells"].as_array().expect("cells");
    let a1 = cells
        .iter()
        .find(|cell| cell["address"] == "A1")
        .expect("A1 in projection");
    assert_eq!(a1["number_format"], "0%");
    assert!(
        a1["style_id"]
            .as_str()
            .expect("style_id")
            .starts_with("st_")
    );
    let a2 = cells
        .iter()
        .find(|cell| cell["address"] == "A2")
        .expect("A2 in projection");
    assert_eq!(a2["number_format"], "$#,##0.00");
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
fn format_handler_caps_large_range_rendering_before_engine_budgeting() {
    let handler = XlsxFormat;
    let model = workbook_model_with_sparse_cells(vec![CellModel {
        element_id: "c_1".into(),
        address: "A1".into(),
        row: 1,
        col: 1,
        value: CellValue::String("only occupied cell".into()),
        formula: None,
        style_id: None,
        number_format: None,
    }]);

    let response = handler
        .read(
            &model,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "range".into(),
                    value: "Revenue!A1:XFD1048576".into(),
                }),
                max_tokens: 1_000,
                continuation: None,
            },
        )
        .expect("read sparse large range");

    assert!(response.content.contains("only occupied cell"));
    assert!(
        response
            .next_actions
            .iter()
            .any(|action| action.contains("truncated")),
        "large ranges should explain how to narrow the request"
    );
}

#[test]
fn format_handler_defaults_to_summary_and_light_preview() {
    let handler = XlsxFormat;
    let cells = (1..=20)
        .map(|row| CellModel {
            element_id: format!("c_{row}"),
            address: format!("A{row}"),
            row,
            col: 1,
            value: CellValue::String(format!("row {row}")),
            formula: None,
            style_id: None,
            number_format: None,
        })
        .collect();
    let model = workbook_model_with_sparse_cells(cells);

    let response = handler
        .read(
            &model,
            &ReadRequest {
                selector: None,
                max_tokens: 1_000,
                continuation: None,
            },
        )
        .expect("read default preview");

    assert!(response.content.contains("# Workbook summary"));
    assert!(response.content.contains("row 10"));
    assert!(!response.content.contains("row 20"));
}

#[test]
fn parser_emits_iso_8601_for_datetime_cells() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("datetime.xlsx");
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    let datetime = ExcelDateTime::from_ymd(2024, 2, 3)
        .expect("date")
        .and_hms(4, 5, 6)
        .expect("time");
    let format = Format::new().set_num_format("yyyy-mm-dd hh:mm:ss");
    worksheet
        .write_datetime_with_format(0, 0, &datetime, &format)
        .expect("datetime");
    workbook.save(&path).expect("workbook fixture");

    let parsed = parse_workbook(&path).expect("parse workbook");

    assert_eq!(
        parsed.sheets[0].cells[0].value,
        CellValue::Datetime("2024-02-03T04:05:06".into())
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

fn workbook_model_with_sparse_cells(cells: Vec<CellModel>) -> dotall_core::ArtifactEnvelope {
    dotall_core::ArtifactEnvelope {
        format_id: "xlsx".into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        payload: serde_json::to_value(WorkbookModel {
            workbook_id: "wb_fixture".into(),
            sheets: vec![SheetModel {
                element_id: "sh_fixture".into(),
                name: "Revenue".into(),
                index: 0,
                dimensions: SheetDimensions {
                    rows: 1_048_576,
                    cols: 16_384,
                },
                merges: Vec::new(),
                cells,
            }],
            named_ranges: Vec::new(),
            style_table: Vec::new(),
            unmodeled: UnmodeledMap {
                charts: PreservationStatus::Preserved,
                pivots: PreservationStatus::Preserved,
                vba: PreservationStatus::Preserved,
                other_ooxml_parts: PreservationStatus::Preserved,
            },
        })
        .expect("serialize model"),
    }
}
