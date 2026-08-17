use std::io::Write;

use dotall_core::{ArtifactEnvelope, SemanticOperation};
use dotall_xlsx::edits::impact::{ImpactOperation, inventory, validate_impact};
use dotall_xlsx::edits::validate_with_source;
use dotall_xlsx::{
    FORMAT_ID, PreservationStatus, SCHEMA_ID, SCHEMA_VERSION, SheetDimensions, SheetModel,
    UnmodeledMap, WorkbookModel,
};
use tempfile::tempdir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

#[test]
fn insert_row_matches_pivot_source_with_case_insensitive_sheet_name() {
    let package = pivot_fixture("PivotData");
    let operation = ImpactOperation::InsertRow {
        sheet: "pivotdata".into(),
        at: 2,
        count: 1,
    };

    let inventory = inventory(&package, &operation).expect("inventory package");
    assert_eq!(inventory.unsupported.len(), 1);
    validate_impact(&package, &operation)
        .expect_err("case-insensitive pivot match must block insert");
}

#[test]
fn insert_row_rejects_when_pivot_source_uses_unresolved_defined_name() {
    let package = pivot_fixture_with_defined_name_source("PivotRange", None);
    let operation = ImpactOperation::InsertRow {
        sheet: "PivotData".into(),
        at: 2,
        count: 1,
    };

    let inventory = inventory(&package, &operation).expect("inventory package");
    assert_eq!(inventory.unsupported.len(), 1);
    assert!(
        inventory.unsupported[0]
            .reason
            .contains("defined name `PivotRange` could not be resolved to a worksheet")
    );

    let error = validate_impact(&package, &operation)
        .expect_err("unresolved defined name must block insert");
    assert!(
        error
            .to_string()
            .contains("defined name `PivotRange` could not be resolved to a worksheet")
    );
}

#[test]
fn insert_row_rejects_when_pivot_source_resolves_defined_name_to_target_sheet() {
    let package =
        pivot_fixture_with_defined_name_source("PivotRange", Some("PivotData!$A$1:$B$10"));
    let operation = ImpactOperation::InsertRow {
        sheet: "pivotdata".into(),
        at: 2,
        count: 1,
    };

    let inventory = inventory(&package, &operation).expect("inventory package");
    assert_eq!(inventory.unsupported.len(), 2);
    assert!(
        inventory
            .unsupported
            .iter()
            .any(|impact| impact.construct == "defined name reference")
    );
    assert!(inventory.unsupported.iter().any(|impact| {
        impact
            .reason
            .contains("changes rows referenced by the pivot cache")
    }));
}

#[test]
fn insert_row_rejects_when_pivot_source_references_target_sheet() {
    let package = pivot_fixture("PivotData");
    let operation = ImpactOperation::InsertRow {
        sheet: "PivotData".into(),
        at: 2,
        count: 1,
    };

    let inventory = inventory(&package, &operation).expect("inventory package");
    assert_eq!(
        inventory
            .unsupported
            .iter()
            .map(|impact| (&impact.part, &impact.construct))
            .collect::<Vec<_>>(),
        vec![(
            &"xl/pivotCache/pivotCacheDefinition1.xml".to_string(),
            &"pivot source reference".to_string(),
        )]
    );

    let error = validate_impact(&package, &operation).expect_err("pivot must block insert");
    assert!(
        error
            .to_string()
            .contains("insert_row is unsafe: unsupported pivot source reference in xl/pivotCache/pivotCacheDefinition1.xml"),
        "unexpected error: {error}"
    );
}

#[test]
fn insert_row_allows_sheet_unrelated_to_pivot_source() {
    let package = pivot_fixture("PivotData");
    let operation = ImpactOperation::InsertRow {
        sheet: "Inputs".into(),
        at: 2,
        count: 1,
    };

    let inventory = inventory(&package, &operation).expect("inventory package");
    assert!(inventory.unsupported.is_empty());
    validate_impact(&package, &operation).expect("unrelated pivot must not block insert");
}

#[test]
fn insert_row_rejects_defined_name_referencing_the_edited_sheet() {
    let package = defined_name_fixture("Inputs!$A$1:$A$10", None);
    let error = validate_impact(
        &package,
        &ImpactOperation::InsertRow {
            sheet: "Inputs".into(),
            at: 2,
            count: 1,
        },
    )
    .expect_err("defined names require a rewrite before structural edits");

    assert!(
        error
            .to_string()
            .contains("unsupported defined name reference in xl/workbook.xml"),
        "unexpected error: {error}"
    );
}

#[test]
fn insert_row_rejects_local_defined_name_on_the_edited_sheet() {
    let package = defined_name_fixture("$A$1:$A$10", Some(0));
    validate_impact(
        &package,
        &ImpactOperation::InsertRow {
            sheet: "Inputs".into(),
            at: 2,
            count: 1,
        },
    )
    .expect_err("local defined names require a rewrite before structural edits");
}

#[test]
fn structural_edits_reject_worksheet_features_and_chart_relationships() {
    let package = structural_feature_fixture();
    let operation = ImpactOperation::InsertRow {
        sheet: "Inputs".into(),
        at: 2,
        count: 1,
    };

    let inventory = inventory(&package, &operation).expect("inventory package");
    let mut constructs = inventory
        .unsupported
        .iter()
        .map(|impact| impact.construct.as_str())
        .collect::<Vec<_>>();
    constructs.sort_unstable();
    assert_eq!(
        constructs,
        vec![
            "autoFilter",
            "chart series references",
            "conditionalFormatting",
            "dataValidations",
            "hyperlinks",
            "table reference",
        ]
    );
    let error = validate_impact(&package, &operation).expect_err("features require rewrite");
    assert!(
        error
            .to_string()
            .contains("insert_row is unsafe: unsupported")
    );
}

#[test]
fn structural_edits_reject_shared_array_and_data_table_formulas() {
    let operation = ImpactOperation::InsertRow {
        sheet: "Inputs".into(),
        at: 2,
        count: 1,
    };

    for formula_type in ["shared", "array", "dataTable"] {
        let package = worksheet_impact_fixture(&format!(
            r#"<sheetData><row r="1"><c r="A1"><f t="{formula_type}" ref="A1:A2">A1</f><v>1</v></c></row></sheetData>"#
        ));
        let inventory = inventory(&package, &operation).expect("inventory package");

        assert!(
            inventory
                .unsupported
                .iter()
                .any(|impact| impact.construct == format!("{formula_type} formula")),
            "{formula_type} formulas must block structural edits: {:#?}",
            inventory.unsupported
        );
        validate_impact(&package, &operation)
            .expect_err("special formula types require an unsupported rewrite");
    }
}

#[test]
fn structural_edits_reject_page_breaks() {
    let package = worksheet_impact_fixture(
        r#"<sheetData/><rowBreaks count="1" manualBreakCount="1"><brk id="1" man="1"/></rowBreaks><colBreaks count="1" manualBreakCount="1"><brk id="1" man="1"/></colBreaks>"#,
    );
    let operation = ImpactOperation::InsertRow {
        sheet: "Inputs".into(),
        at: 2,
        count: 1,
    };

    let inventory = inventory(&package, &operation).expect("inventory package");
    let constructs = inventory
        .unsupported
        .iter()
        .map(|impact| impact.construct.as_str())
        .collect::<Vec<_>>();

    assert!(constructs.contains(&"rowBreaks"));
    assert!(constructs.contains(&"colBreaks"));
    validate_impact(&package, &operation).expect_err("page breaks require an unsupported rewrite");
}

#[test]
fn delete_row_rejects_pivot_source_references() {
    let package = pivot_fixture("PivotData");
    let error = validate_impact(
        &package,
        &ImpactOperation::DeleteRow {
            sheet: "PivotData".into(),
            at: 2,
            count: 1,
        },
    )
    .expect_err("pivot must block delete");
    assert!(error.to_string().contains("delete_row is unsafe"));
}

#[test]
fn source_aware_validation_rejects_before_structural_edit_is_staged() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("pivot.xlsx");
    std::fs::write(&source, pivot_fixture("PivotData")).expect("write fixture");
    let model = ArtifactEnvelope {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        payload: serde_json::to_value(WorkbookModel {
            workbook_id: "wb_fixture".into(),
            sheets: vec![fixture_sheet("Inputs"), fixture_sheet("PivotData")],
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
    };

    let error = validate_with_source(
        &source,
        &model,
        &[SemanticOperation {
            kind: "insert_row".into(),
            payload: serde_json::json!({ "sheet": "PivotData", "at": 2, "count": 1 }),
        }],
    )
    .expect_err("unsafe structural edit must not stage");

    assert!(
        error.to_string().contains(
            "unsupported pivot source reference in xl/pivotCache/pivotCacheDefinition1.xml"
        )
    );
}

fn fixture_sheet(name: &str) -> SheetModel {
    SheetModel {
        element_id: format!("sh_{name}"),
        name: name.into(),
        index: 0,
        dimensions: SheetDimensions { rows: 10, cols: 2 },
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
        cells: Vec::new(),
    }
}

fn pivot_fixture(pivot_source_sheet: &str) -> Vec<u8> {
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    for (path, xml) in pivot_fixture_entries(
        &format!(
            r#"<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cacheSource type="worksheet"><worksheetSource sheet="{pivot_source_sheet}" ref="A1:B10"/></cacheSource></pivotCacheDefinition>"#
        ),
        None,
    ) {
        writer
            .start_file(path, options)
            .expect("start fixture entry");
        writer
            .write_all(xml.as_bytes())
            .expect("write fixture entry");
    }
    writer.finish().expect("finish fixture").into_inner()
}

fn pivot_fixture_with_defined_name_source(
    defined_name: &str,
    defined_name_formula: Option<&str>,
) -> Vec<u8> {
    let pivot_cache = format!(
        r#"<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cacheSource type="worksheet"><worksheetSource name="{defined_name}" ref="A1:B10"/></cacheSource></pivotCacheDefinition>"#
    );
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    for (path, xml) in pivot_fixture_entries(&pivot_cache, defined_name_formula) {
        writer
            .start_file(path, options)
            .expect("start fixture entry");
        writer
            .write_all(xml.as_bytes())
            .expect("write fixture entry");
    }
    writer.finish().expect("finish fixture").into_inner()
}

fn pivot_fixture_entries(
    pivot_cache_xml: &str,
    defined_name_formula: Option<&str>,
) -> [(&'static str, String); 5] {
    let workbook = match defined_name_formula {
        Some(formula) => format!(
            r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Inputs" sheetId="1" r:id="rId1"/><sheet name="PivotData" sheetId="2" r:id="rId2"/></sheets><definedNames><definedName name="PivotRange">{formula}</definedName></definedNames><pivotCaches><pivotCache cacheId="1" r:id="rId3"/></pivotCaches></workbook>"#
        ),
        None => r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Inputs" sheetId="1" r:id="rId1"/><sheet name="PivotData" sheetId="2" r:id="rId2"/></sheets><pivotCaches><pivotCache cacheId="1" r:id="rId3"/></pivotCaches></workbook>"#.into(),
    };
    [
        ("xl/workbook.xml", workbook),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition" Target="pivotCache/pivotCacheDefinition1.xml"/></Relationships>"#.into(),
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.into(),
        ),
        (
            "xl/worksheets/sheet2.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.into(),
        ),
        ("xl/pivotCache/pivotCacheDefinition1.xml", pivot_cache_xml.into()),
    ]
}

fn defined_name_fixture(formula: &str, local_sheet_id: Option<u32>) -> Vec<u8> {
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    for (path, xml) in [
        (
            "xl/workbook.xml",
            format!(
                r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Inputs" sheetId="1" r:id="rId1"/><sheet name="Summary" sheetId="2" r:id="rId2"/></sheets><definedNames><definedName name="InputRange"{}>{formula}</definedName></definedNames></workbook>"#,
                local_sheet_id.map(|id| format!(r#" localSheetId="{id}""#)).unwrap_or_default(),
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#.into(),
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.into(),
        ),
        (
            "xl/worksheets/sheet2.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.into(),
        ),
    ] {
        writer
            .start_file(path, options)
            .expect("start fixture entry");
        writer
            .write_all(xml.as_bytes())
            .expect("write fixture entry");
    }
    writer.finish().expect("finish fixture").into_inner()
}

fn structural_feature_fixture() -> Vec<u8> {
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    for (path, xml) in [
        (
            "xl/workbook.xml",
            r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Inputs" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<worksheet xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetData/><autoFilter ref="A1:B2"/><dataValidations/><conditionalFormatting sqref="A1"/><hyperlinks/><tableParts><tablePart r:id="rId1"/></tableParts><drawing r:id="rId2"/></worksheet>"#,
        ),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#,
        ),
        (
            "xl/drawings/_rels/drawing1.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>"#,
        ),
    ] {
        writer
            .start_file(path, options)
            .expect("start fixture entry");
        writer
            .write_all(xml.as_bytes())
            .expect("write fixture entry");
    }
    writer.finish().expect("finish fixture").into_inner()
}

fn worksheet_impact_fixture(worksheet_contents: &str) -> Vec<u8> {
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    for (path, xml) in [
        (
            "xl/workbook.xml",
            r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Inputs" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            &format!(
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{worksheet_contents}</worksheet>"#
            ),
        ),
    ] {
        writer
            .start_file(path, options)
            .expect("start fixture entry");
        writer
            .write_all(xml.as_bytes())
            .expect("write fixture entry");
    }
    writer.finish().expect("finish fixture").into_inner()
}
