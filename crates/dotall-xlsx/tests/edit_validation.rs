use dotall_core::{ArtifactEnvelope, SemanticOperation};
use dotall_xlsx::edits::{SCHEMA_ID, validate};
use dotall_xlsx::{
    CellModel, CellValue, FORMAT_ID, PreservationStatus, SheetDimensions, SheetModel, UnmodeledMap,
    WorkbookModel,
};

#[test]
fn validates_set_cell_formula_with_semantic_diff() {
    let validated = validate(
        &envelope(workbook_fixture()),
        &[SemanticOperation {
            kind: "set_cell_formula".into(),
            payload: serde_json::json!({
                "sheet": "Summary",
                "address": "B2",
                "formula": "Inputs!A1*1.1"
            }),
        }],
    )
    .expect("valid edit");

    assert_eq!(validated.format_id, FORMAT_ID);
    assert_eq!(validated.schema_id, SCHEMA_ID);
    assert_eq!(validated.schema_version, 1);
    assert_eq!(
        validated.semantic_diff[0].before.as_deref(),
        Some("=Inputs!A1")
    );
    assert_eq!(
        validated.semantic_diff[0].after.as_deref(),
        Some("=Inputs!A1*1.1")
    );
}

#[test]
fn validates_set_cell_value_with_dependency_impact() {
    let validated = validate(
        &envelope(workbook_fixture()),
        &[SemanticOperation {
            kind: "set_cell_value".into(),
            payload: serde_json::json!({
                "sheet": "Inputs",
                "address": "A1",
                "value": 100
            }),
        }],
    )
    .expect("valid edit");

    assert_eq!(validated.semantic_diff[0].change, "value");
    assert_eq!(validated.semantic_diff[0].after.as_deref(), Some("100"));
    assert_eq!(
        validated.dependency_impact.forward,
        vec!["Summary!B2".to_string(), "Summary!C2".to_string()]
    );
    assert_eq!(
        validated.dependency_impact.notes,
        vec!["refs parsed; values not evaluated".to_string()]
    );
}

#[test]
fn rejects_unknown_sheet() {
    let error = validate(
        &envelope(workbook_fixture()),
        &[SemanticOperation {
            kind: "set_cell_value".into(),
            payload: serde_json::json!({
                "sheet": "Revenue",
                "address": "A1",
                "value": 1
            }),
        }],
    )
    .expect_err("unknown sheet");

    let message = error.to_string();
    assert!(message.contains("unknown sheet `Revenue`"));
    assert!(message.contains("Inputs"));
}

fn envelope(model: WorkbookModel) -> ArtifactEnvelope {
    ArtifactEnvelope {
        format_id: FORMAT_ID.into(),
        schema_id: "xlsx.workbook".into(),
        schema_version: 1,
        payload: serde_json::to_value(model).expect("serialize workbook"),
    }
}

fn workbook_fixture() -> WorkbookModel {
    WorkbookModel {
        workbook_id: "wb_fixture".into(),
        sheets: vec![
            fixture_sheet(
                "Inputs",
                vec![
                    fixture_cell("c_inputs_a1", "A1", None),
                    fixture_cell("c_inputs_b1", "B1", None),
                ],
            ),
            fixture_sheet(
                "Summary",
                vec![
                    fixture_cell("c_summary_b2", "B2", Some("=Inputs!A1")),
                    fixture_cell("c_summary_c2", "C2", Some("=SUM(Inputs!A1:A10)")),
                ],
            ),
        ],
        named_ranges: Vec::new(),
        style_table: Vec::new(),
        unmodeled: UnmodeledMap {
            charts: PreservationStatus::Preserved,
            pivots: PreservationStatus::Preserved,
            vba: PreservationStatus::Preserved,
            other_ooxml_parts: PreservationStatus::Preserved,
        },
    }
}

fn fixture_sheet(name: &str, cells: Vec<CellModel>) -> SheetModel {
    SheetModel {
        element_id: format!("sh_{name}"),
        name: name.into(),
        index: 0,
        dimensions: SheetDimensions { rows: 10, cols: 4 },
        merges: Vec::new(),
        freeze_panes: None,
        cells,
    }
}

fn fixture_cell(element_id: &str, address: &str, formula: Option<&str>) -> CellModel {
    let split = address
        .find(|character: char| character.is_ascii_digit())
        .expect("address");
    let (column, row) = address.split_at(split);
    CellModel {
        element_id: element_id.into(),
        address: address.into(),
        row: row.parse().expect("row"),
        col: u32::from(column.as_bytes()[0] - b'A' + 1),
        value: CellValue::Empty,
        formula: formula.map(str::to_owned),
        style_id: None,
        number_format: None,
    }
}
