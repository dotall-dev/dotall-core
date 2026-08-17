use std::fs;

use dotall_core::{ArtifactEnvelope, DotallStore};
use dotall_xlsx::dependencies::{
    DependencyTarget, build, ensure_formula_dependencies, to_artifact,
};
use dotall_xlsx::{
    CellModel, CellValue, NamedRange, PreservationStatus, SheetDimensions, SheetModel,
    UnmodeledMap, WorkbookModel,
};
use tempfile::tempdir;

#[test]
fn builds_forward_and_reverse_dependencies_without_expanding_ranges() {
    let model = workbook_fixture();
    let graph = build(&model);

    let summary = sheet(&model, "Summary");
    let inputs = sheet(&model, "Inputs");
    let sink = cell(summary, "D2");
    let direct_dependent = cell(summary, "B2");
    let range_dependent = cell(summary, "C2");
    let sink_input = cell(summary, "C2");
    let source = cell(inputs, "A1");
    let named_range_source = cell(inputs, "B1");

    assert!(graph.forward(&sink.element_id).iter().any(|edge| {
        matches!(
            &edge.to,
            DependencyTarget::Element { element_id } if element_id == &named_range_source.element_id
        )
    }));
    assert!(graph.forward(&sink.element_id).iter().any(|edge| {
        matches!(
            &edge.to,
            DependencyTarget::Element { element_id } if element_id == &sink_input.element_id
        )
    }));
    assert!(
        graph
            .forward(&range_dependent.element_id)
            .iter()
            .any(|edge| {
                matches!(
                    &edge.to,
                    DependencyTarget::Selector { selector } if selector == "Inputs!A1:A10"
                )
            })
    );
    assert_eq!(graph.forward(&range_dependent.element_id).len(), 1);

    let reverse = graph.reverse(&source.element_id);
    assert_eq!(reverse.len(), 2);
    assert!(
        reverse
            .iter()
            .any(|edge| edge.from_element_id == direct_dependent.element_id)
    );
    assert!(
        reverse
            .iter()
            .any(|edge| edge.from_element_id == range_dependent.element_id)
    );

    let sparse_reverse = graph.reverse_at("Inputs", "A5");
    assert_eq!(sparse_reverse.len(), 1);
    assert_eq!(
        sparse_reverse[0].from_element_id,
        range_dependent.element_id
    );
}

#[test]
fn ignores_external_workbook_formula_references() {
    let model = WorkbookModel {
        workbook_id: "wb_external".into(),
        sheets: vec![
            fixture_sheet("Inputs", vec![fixture_cell("c_inputs_a1", "A1", None)]),
            fixture_sheet(
                "Summary",
                vec![fixture_cell(
                    "c_summary_b2",
                    "B2",
                    Some("=[Other.xlsx]Inputs!A1"),
                )],
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
    };
    let graph = build(&model);

    let summary = sheet(&model, "Summary");
    let inputs = sheet(&model, "Inputs");
    let dependent = cell(summary, "B2");
    let source = cell(inputs, "A1");

    assert!(graph.forward(&dependent.element_id).is_empty());
    assert!(graph.reverse(&source.element_id).is_empty());
}

#[test]
fn resolves_sheet_references_case_insensitively() {
    let model = WorkbookModel {
        workbook_id: "wb_case".into(),
        sheets: vec![
            fixture_sheet("Inputs", vec![fixture_cell("c_inputs_a1", "A1", None)]),
            fixture_sheet(
                "Summary",
                vec![fixture_cell("c_summary_b2", "B2", Some("=inputs!A1"))],
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
    };
    let graph = build(&model);

    let inputs = sheet(&model, "Inputs");
    let summary = sheet(&model, "Summary");
    let source = cell(inputs, "A1");
    let dependent = cell(summary, "B2");

    assert!(graph.forward(&dependent.element_id).iter().any(|edge| {
        matches!(
            &edge.to,
            DependencyTarget::Element { element_id } if element_id == &source.element_id
        )
    }));
    assert_eq!(graph.reverse(&source.element_id).len(), 1);
    assert_eq!(
        graph.reverse(&source.element_id)[0].from_element_id,
        dependent.element_id
    );
}

#[test]
fn serializes_the_graph_as_a_formula_dependencies_artifact() {
    let graph = build(&workbook_fixture());

    let artifact = to_artifact(&graph);

    assert_eq!(artifact.format_id, "xlsx");
    assert_eq!(artifact.schema_id, "xlsx.formula-dependencies");
    assert_eq!(artifact.schema_version, 1);
    assert_eq!(
        artifact.payload["edges"],
        serde_json::to_value(graph.edges).expect("serialize edges")
    );
}

#[test]
fn formula_dependency_cache_hits_for_identical_model_and_rebuilds_when_source_changes() {
    let temp = tempdir().expect("workspace");
    let path = temp.path().join("book.xlsx");
    fs::write(&path, b"first source").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("store");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register first source");
    let first_model = artifact_for(workbook_fixture());
    let first_hash = source_hash(&store);

    let (_, first_cache_hit) =
        ensure_formula_dependencies(&store, "book.xlsx", &first_model, &first_hash)
            .expect("build dependencies");
    let (_, second_cache_hit) =
        ensure_formula_dependencies(&store, "book.xlsx", &first_model, &first_hash)
            .expect("read cached dependencies");

    assert!(!first_cache_hit);
    assert!(second_cache_hit);

    fs::write(&path, b"changed source").expect("replace source");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register changed source");
    let changed_model = artifact_for(WorkbookModel {
        workbook_id: "wb_changed".into(),
        ..workbook_fixture()
    });

    let (_, rebuilt_cache_hit) =
        ensure_formula_dependencies(&store, "book.xlsx", &changed_model, &source_hash(&store))
            .expect("rebuild dependencies");

    assert!(!rebuilt_cache_hit);
}

fn artifact_for(model: WorkbookModel) -> ArtifactEnvelope {
    ArtifactEnvelope {
        format_id: "xlsx".into(),
        schema_id: "xlsx.workbook".into(),
        schema_version: 1,
        payload: serde_json::to_value(model).expect("serialize workbook"),
    }
}

fn source_hash(store: &DotallStore) -> String {
    store.manifest().objects["book.xlsx"]
        .fingerprint
        .blake3
        .clone()
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
                    fixture_cell("c_summary_d2", "D2", Some("=TaxRate * C2")),
                ],
            ),
        ],
        named_ranges: vec![NamedRange {
            element_id: "nr_tax_rate".into(),
            name: "TaxRate".into(),
            formula: "=Inputs!$B$1".into(),
        }],
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
        tab_color: None,
        auto_filter: None,
        print_area: None,
        print_titles: None,
        page_orientation: None,
        print_scale: None,
        fit_to_page: None,
        center_on_page: None,
        page_margins: None,
        cells,
    }
}

fn fixture_cell(element_id: &str, address: &str, formula: Option<&str>) -> CellModel {
    let (column, row) = address.split_at(1);
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

fn sheet<'a>(model: &'a WorkbookModel, name: &str) -> &'a SheetModel {
    model
        .sheets
        .iter()
        .find(|sheet| sheet.name == name)
        .expect("sheet")
}

fn cell<'a>(sheet: &'a SheetModel, address: &str) -> &'a CellModel {
    sheet
        .cells
        .iter()
        .find(|cell| cell.address == address)
        .expect("cell")
}
