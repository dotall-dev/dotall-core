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
        error
            .to_string()
            .contains("unsupported pivot source reference in xl/pivotCache/pivotCacheDefinition1.xml")
    );
}

fn fixture_sheet(name: &str) -> SheetModel {
    SheetModel {
        element_id: format!("sh_{name}"),
        name: name.into(),
        index: 0,
        dimensions: SheetDimensions { rows: 10, cols: 2 },
        merges: Vec::new(),
        cells: Vec::new(),
    }
}

fn pivot_fixture(pivot_source_sheet: &str) -> Vec<u8> {
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    for (path, xml) in [
        (
            "xl/workbook.xml",
            r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Inputs" sheetId="1" r:id="rId1"/><sheet name="PivotData" sheetId="2" r:id="rId2"/></sheets><pivotCaches><pivotCache cacheId="1" r:id="rId3"/></pivotCaches></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition" Target="pivotCache/pivotCacheDefinition1.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#,
        ),
        (
            "xl/worksheets/sheet2.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#,
        ),
        (
            "xl/pivotCache/pivotCacheDefinition1.xml",
            &format!(
                r#"<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cacheSource type="worksheet"><worksheetSource sheet="{pivot_source_sheet}" ref="A1:B10"/></cacheSource></pivotCacheDefinition>"#
            ),
        ),
    ] {
        writer.start_file(path, options).expect("start fixture entry");
        writer.write_all(xml.as_bytes()).expect("write fixture entry");
    }
    writer.finish().expect("finish fixture").into_inner()
}
