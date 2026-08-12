use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};

use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::edits::validate_with_source;
use dotall_xlsx::{XlsxFormat, parse_workbook};
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

#[test]
fn patches_a_cell_value_without_changing_other_zip_entries() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A1",
                    "value": 42,
                }),
            }],
        )
        .expect("validate value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply value edit");
    let after_model = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(cell_value(&after_model, "Inputs", "A1"), "42");
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn patches_string_cells_through_shared_strings_and_reuses_existing_entries() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_shared_string_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");
    assert!(
        zip_entries(&before).contains_key("xl/sharedStrings.xml"),
        "fixture must include shared strings"
    );

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[
                SemanticOperation {
                    kind: "set_cell_value".into(),
                    payload: serde_json::json!({
                        "sheet": "Inputs",
                        "address": "A1",
                        "value": "appended",
                    }),
                },
                SemanticOperation {
                    kind: "set_cell_value".into(),
                    payload: serde_json::json!({
                        "sheet": "Inputs",
                        "address": "B1",
                        "value": "existing",
                    }),
                },
            ],
        )
        .expect("validate string edits");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply string edits");
    let worksheet = worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    let shared_strings = worksheet_xml(&patched.bytes, "xl/sharedStrings.xml");

    assert!(worksheet.contains(r#"<c r="A1" t="s"><v>1</v></c>"#));
    assert!(worksheet.contains(r#"<c r="B1" t="s"><v>0</v></c>"#));
    assert!(shared_strings.contains(r#"count="2" uniqueCount="2""#));
    assert!(shared_strings.contains("<si><t>existing</t></si>"));
    assert!(shared_strings.contains("<si><t>appended</t></si>"));
    assert_untouched_entries_are_identical(
        &before,
        &patched.bytes,
        &["xl/worksheets/sheet1.xml", "xl/sharedStrings.xml"],
    );
}

#[test]
fn patches_string_cells_inline_when_workbook_has_no_shared_strings() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");
    assert!(
        !zip_entries(&before).contains_key("xl/sharedStrings.xml"),
        "fixture must not include shared strings"
    );

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A1",
                    "value": "inline",
                }),
            }],
        )
        .expect("validate string edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply string edit");

    assert!(
        worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml")
            .contains(r#"<c r="A1" t="inlineStr"><is><t>inline</t></is></c>"#)
    );
    assert!(!zip_entries(&patched.bytes).contains_key("xl/sharedStrings.xml"));
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn patches_a_formula_without_changing_other_worksheet() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_formula".into(),
                payload: serde_json::json!({
                    "sheet": "Summary",
                    "address": "B1",
                    "formula": "Inputs!A1*2",
                }),
            }],
        )
        .expect("validate formula edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply formula edit");
    let after_model = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(formula(&after_model, "Summary", "B1"), "=Inputs!A1*2");
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet2.xml"]);
}

#[test]
fn adds_a_sparse_cell_inside_a_valid_worksheet_row() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B2",
                    "value": "created",
                }),
            }],
        )
        .expect("validate sparse value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply sparse value edit");
    let after_model = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(cell_value(&after_model, "Inputs", "B2"), "created");
    assert!(
        worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml")
            .contains(r#"<row r="2"><c r="B2""#),
        "new cells must be nested in their worksheet row"
    );
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn adds_a_cell_to_an_existing_worksheet_row() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B1",
                    "value": "adjacent",
                }),
            }],
        )
        .expect("validate adjacent value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply adjacent value edit");

    assert_eq!(
        cell_value(
            &parse_workbook_bytes(&patched.bytes, directory.path()),
            "Inputs",
            "B1"
        ),
        "adjacent"
    );
    let xml = worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml");
    assert_eq!(xml.matches(r#"<row r="1""#).count(), 1);
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn resolves_xml_escaped_sheet_names() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    let mut workbook = Workbook::new();
    workbook
        .add_worksheet()
        .set_name("Sales & Ops")
        .expect("sheet name")
        .write_number(0, 0, 1)
        .expect("cell value");
    workbook.save(&source).expect("write fixture");
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Sales & Ops",
                    "address": "A1",
                    "value": 42,
                }),
            }],
        )
        .expect("validate value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply value edit");

    assert_eq!(
        cell_value(
            &parse_workbook_bytes(&patched.bytes, directory.path()),
            "Sales & Ops",
            "A1"
        ),
        "42"
    );
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn adds_a_cell_to_a_row_with_attributes_before_its_reference() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    replace_zip_entry(&source, "xl/worksheets/sheet1.xml", |xml| {
        xml.replacen(r#"<row r="1""#, r#"<row s="1" r="1""#, 1)
    });
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "B1",
                    "value": "adjacent",
                }),
            }],
        )
        .expect("validate value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply value edit");
    let xml = worksheet_xml(&patched.bytes, "xl/worksheets/sheet1.xml");

    assert_eq!(xml.matches(r#"<row "#).count(), 1);
    assert!(xml.contains(r#"<row s="1" r="1""#));
    assert!(xml.contains(r#"<c r="B1" t="inlineStr""#));
    assert_untouched_entries_are_identical(&before, &patched.bytes, &["xl/worksheets/sheet1.xml"]);
}

#[test]
fn rejects_edits_to_shared_formula_cells() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);
    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Summary",
                    "address": "B1",
                    "value": 42,
                }),
            }],
        )
        .expect("validate value edit");
    replace_zip_entry(&source, "xl/worksheets/sheet2.xml", |xml| {
        xml.replacen(
            "<f>Inputs!A1</f>",
            r#"<f t="shared" si="0" ref="B1:B2">Inputs!A1</f>"#,
            1,
        )
    });

    let error = handler
        .apply_edit(&source, &edit)
        .expect_err("shared formula edits must be rejected");

    assert!(
        error.to_string().contains("shared formula"),
        "unexpected error: {error}"
    );
}

#[test]
fn insert_row_only_shifts_the_edited_sheet_and_updates_cross_sheet_formulas() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_structural_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = validate_with_source(
        &source,
        &model,
        &[SemanticOperation {
            kind: "insert_row".into(),
            payload: serde_json::json!({ "sheet": "Inputs", "at": 2, "count": 1 }),
        }],
    )
    .expect("validate row insert");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply row insert");
    let after = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(cell_value(&after, "Inputs", "A4"), "30");
    assert_eq!(cell_value(&after, "Summary", "A3"), "99");
    assert_eq!(formula(&after, "Summary", "B3"), "=Inputs!A4");
    assert_untouched_entries_are_identical(
        &before,
        &patched.bytes,
        &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
    );
}

#[test]
fn delete_row_preserves_unrelated_zip_entries() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_structural_fixture(&source);
    let before = fs::read(&source).expect("fixture bytes");

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = validate_with_source(
        &source,
        &model,
        &[SemanticOperation {
            kind: "delete_row".into(),
            payload: serde_json::json!({ "sheet": "Inputs", "at": 2, "count": 1 }),
        }],
    )
    .expect("validate row delete");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply row delete");
    let after = parse_workbook_bytes(&patched.bytes, directory.path());

    assert_eq!(cell_value(&after, "Inputs", "A2"), "30");
    assert_eq!(formula(&after, "Summary", "B3"), "=Inputs!A2");
    assert_untouched_entries_are_identical(
        &before,
        &patched.bytes,
        &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
    );
}

#[test]
fn delete_row_removes_the_calc_chain_relationship_with_the_calc_chain() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_structural_fixture(&source);
    replace_zip_entry(&source, "xl/_rels/workbook.xml.rels", |xml| {
        xml.replacen(
            "</Relationships>",
            r#"<Relationship Id="rId99" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/></Relationships>"#,
            1,
        )
    });
    replace_zip_entry(&source, "[Content_Types].xml", |xml| {
        xml.replacen(
            "</Types>",
            r#"<Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/></Types>"#,
            1,
        )
    });
    append_zip_entry(
        &source,
        "xl/calcChain.xml",
        r#"<calcChain xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#,
    );

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = validate_with_source(
        &source,
        &model,
        &[SemanticOperation {
            kind: "delete_row".into(),
            payload: serde_json::json!({ "sheet": "Inputs", "at": 2, "count": 1 }),
        }],
    )
    .expect("validate row delete");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply row delete");
    let entries = zip_entries(&patched.bytes);

    assert!(!entries.contains_key("xl/calcChain.xml"));
    assert!(!worksheet_xml(&patched.bytes, "xl/_rels/workbook.xml.rels").contains("calcChain"));
    assert!(!worksheet_xml(&patched.bytes, "[Content_Types].xml").contains("calcChain"));
}

#[test]
fn value_edits_force_excel_recalculation_on_open() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    // Simulate an Excel-saved workbook: stale formula cache + no fullCalcOnLoad + calcChain.
    replace_zip_entry(&source, "xl/workbook.xml", |xml| {
        if let Some(start) = xml.find("<calcPr") {
            let end = xml[start..]
                .find('>')
                .map(|offset| start + offset + 1)
                .expect("calcPr tag");
            format!(
                "{}{}{}",
                &xml[..start],
                r#"<calcPr calcId="191029" calcMode="auto" fullCalcOnLoad="0"/>"#,
                &xml[end..]
            )
        } else {
            xml.replacen(
                "</workbook>",
                r#"<calcPr calcId="191029" calcMode="auto" fullCalcOnLoad="0"/></workbook>"#,
                1,
            )
        }
    });
    replace_zip_entry(&source, "xl/worksheets/sheet2.xml", |xml| {
        xml.replacen("<f>Inputs!A1</f>", "<f>Inputs!A1</f><v>1</v>", 1)
    });
    replace_zip_entry(&source, "xl/_rels/workbook.xml.rels", |xml| {
        xml.replacen(
            "</Relationships>",
            r#"<Relationship Id="rId99" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/></Relationships>"#,
            1,
        )
    });
    replace_zip_entry(&source, "[Content_Types].xml", |xml| {
        xml.replacen(
            "</Types>",
            r#"<Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/></Types>"#,
            1,
        )
    });
    append_zip_entry(
        &source,
        "xl/calcChain.xml",
        r#"<calcChain xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><c r="B1" i="2"/></calcChain>"#,
    );

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A1",
                    "value": 99,
                }),
            }],
        )
        .expect("validate value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply value edit");
    let workbook_xml = worksheet_xml(&patched.bytes, "xl/workbook.xml");
    let summary_xml = worksheet_xml(&patched.bytes, "xl/worksheets/sheet2.xml");
    let entries = zip_entries(&patched.bytes);

    assert!(
        workbook_xml.contains(r#"fullCalcOnLoad="1""#),
        "value edits must mark the workbook for full recalculation on open: {workbook_xml}"
    );
    assert!(
        workbook_xml.contains(r#"forceFullCalc="1""#),
        "value edits must force full calc so dependents refresh: {workbook_xml}"
    );
    assert!(
        !summary_xml.contains("<v>1</v>"),
        "stale formula caches on untouched sheets must be stripped: {summary_xml}"
    );
    assert!(
        summary_xml.contains("<f>Inputs!A1</f>"),
        "formula text must be preserved: {summary_xml}"
    );
    assert!(!entries.contains_key("xl/calcChain.xml"));
    assert!(!worksheet_xml(&patched.bytes, "xl/_rels/workbook.xml.rels").contains("calcChain"));
    assert!(!worksheet_xml(&patched.bytes, "[Content_Types].xml").contains("calcChain"));
    assert_eq!(
        cell_value(
            &parse_workbook_bytes(&patched.bytes, directory.path()),
            "Inputs",
            "A1"
        ),
        "99"
    );
}

#[test]
fn value_edits_upgrade_existing_full_calc_on_load_and_strip_caches() {
    // YC demo failure mode: generated workbooks already ship with fullCalcOnLoad="1",
    // so a no-op early return left stale Board/chart formula caches intact.
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("workbook.xlsx");
    write_fixture(&source);

    replace_zip_entry(&source, "xl/workbook.xml", |xml| {
        if let Some(start) = xml.find("<calcPr") {
            let end = xml[start..]
                .find('>')
                .map(|offset| start + offset + 1)
                .expect("calcPr tag");
            format!(
                "{}{}{}",
                &xml[..start],
                r#"<calcPr calcId="124519" fullCalcOnLoad="1"/>"#,
                &xml[end..]
            )
        } else {
            xml.replacen(
                "</workbook>",
                r#"<calcPr calcId="124519" fullCalcOnLoad="1"/></workbook>"#,
                1,
            )
        }
    });
    replace_zip_entry(&source, "xl/worksheets/sheet2.xml", |xml| {
        xml.replacen("<f>Inputs!A1</f>", "<f>Inputs!A1</f><v>49</v>", 1)
    });

    let handler = XlsxFormat;
    let model = handler.parse(&source).expect("parse fixture");
    let edit = handler
        .validate_edit(
            &model,
            &[SemanticOperation {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "address": "A1",
                    "value": 59,
                }),
            }],
        )
        .expect("validate value edit");

    let patched = handler
        .apply_edit(&source, &edit)
        .expect("apply value edit");
    let workbook_xml = worksheet_xml(&patched.bytes, "xl/workbook.xml");
    let summary_xml = worksheet_xml(&patched.bytes, "xl/worksheets/sheet2.xml");

    assert!(
        workbook_xml.contains(r#"fullCalcOnLoad="1""#)
            && workbook_xml.contains(r#"forceFullCalc="1""#),
        "must upgrade calcPr even when fullCalcOnLoad was already set: {workbook_xml}"
    );
    assert!(
        !summary_xml.contains("<v>49</v>"),
        "dependent formula cache must be cleared: {summary_xml}"
    );
    assert!(summary_xml.contains("<f>Inputs!A1</f>"));
}

fn write_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("sheet name");
    inputs.write_number(0, 0, 1).expect("input value");
    let summary = workbook
        .add_worksheet()
        .set_name("Summary")
        .expect("sheet name");
    summary
        .write_formula(0, 1, "=Inputs!A1")
        .expect("summary formula");
    workbook.save(path).expect("write fixture");
}

fn write_structural_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("sheet name");
    inputs.write_number(0, 0, 10).expect("input value");
    inputs.write_number(2, 0, 30).expect("input value");
    let summary = workbook
        .add_worksheet()
        .set_name("Summary")
        .expect("sheet name");
    summary.write_number(2, 0, 99).expect("summary value");
    summary
        .write_formula(2, 1, "=Inputs!A3")
        .expect("summary formula");
    workbook.save(path).expect("write fixture");
}

fn write_shared_string_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let inputs = workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("sheet name");
    inputs.write_string(0, 0, "existing").expect("string value");
    inputs.write_number(0, 1, 1).expect("number value");
    workbook.save(path).expect("write fixture");
}

fn parse_workbook_bytes(bytes: &[u8], directory: &std::path::Path) -> dotall_xlsx::WorkbookModel {
    let path = directory.join("patched.xlsx");
    fs::write(&path, bytes).expect("write patched workbook");
    parse_workbook(&path).expect("parse patched workbook")
}

fn cell_value(workbook: &dotall_xlsx::WorkbookModel, sheet: &str, address: &str) -> String {
    let cell = workbook
        .sheets
        .iter()
        .find(|candidate| candidate.name == sheet)
        .and_then(|candidate| candidate.cells.iter().find(|cell| cell.address == address))
        .expect("cell");
    dotall_xlsx::edits::format_cell_value(&cell.value).expect("cell value")
}

fn formula(workbook: &dotall_xlsx::WorkbookModel, sheet: &str, address: &str) -> String {
    workbook
        .sheets
        .iter()
        .find(|candidate| candidate.name == sheet)
        .and_then(|candidate| candidate.cells.iter().find(|cell| cell.address == address))
        .and_then(|cell| cell.formula.clone())
        .expect("formula")
}

#[derive(Debug, PartialEq, Eq)]
struct ZipEntrySnapshot {
    crc32: u32,
    compression: zip::CompressionMethod,
    compressed_size: u64,
    inflated: Vec<u8>,
    raw_compressed: Vec<u8>,
}

fn assert_untouched_entries_are_identical(before: &[u8], after: &[u8], patched: &[&str]) {
    let before_entries = zip_entries(before);
    let after_entries = zip_entries(after);

    assert_eq!(
        before_entries.len(),
        after_entries.len(),
        "ZIP entry count changed"
    );
    assert!(
        before_entries.contains_key("xl/styles.xml"),
        "fixture must include styles.xml"
    );
    for (name, before_entry) in &before_entries {
        let after_entry = after_entries.get(name).expect("entry retained");
        if patched.contains(&name.as_str()) || is_calc_invalidation_part(name) {
            continue;
        }
        assert_eq!(
            before_entry.crc32, after_entry.crc32,
            "CRC changed for {name}"
        );
        assert_eq!(
            before_entry.compression, after_entry.compression,
            "compression method changed for {name}"
        );
        assert_eq!(
            before_entry.compressed_size, after_entry.compressed_size,
            "compressed size changed for {name}"
        );
        assert_eq!(
            before_entry.inflated, after_entry.inflated,
            "inflated bytes changed for {name}"
        );
        assert_eq!(
            before_entry.raw_compressed, after_entry.raw_compressed,
            "raw compressed payload changed for {name}"
        );
    }
}

/// Parts rewritten so spreadsheet apps recalculate dependents/charts on open.
fn is_calc_invalidation_part(name: &str) -> bool {
    name == "xl/workbook.xml"
        || name.starts_with("xl/worksheets/")
        || name.starts_with("xl/charts/")
}

fn zip_entries(bytes: &[u8]) -> BTreeMap<String, ZipEntrySnapshot> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open ZIP");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let name = {
            let entry = archive.by_index(index).expect("ZIP entry");
            entry.name().to_owned()
        };
        let (crc32, compression, compressed_size) = {
            let entry = archive.by_index(index).expect("ZIP entry");
            (entry.crc32(), entry.compression(), entry.compressed_size())
        };
        let mut raw_compressed = Vec::new();
        archive
            .by_index_raw(index)
            .expect("raw ZIP entry")
            .read_to_end(&mut raw_compressed)
            .expect("read raw compressed payload");
        let mut inflated = Vec::new();
        archive
            .by_index(index)
            .expect("ZIP entry")
            .read_to_end(&mut inflated)
            .expect("read inflated ZIP entry");
        entries.insert(
            name,
            ZipEntrySnapshot {
                crc32,
                compression,
                compressed_size,
                inflated,
                raw_compressed,
            },
        );
    }
    entries
}

fn worksheet_xml(bytes: &[u8], name: &str) -> String {
    let entries = zip_entries(bytes);
    String::from_utf8(entries[name].inflated.clone()).expect("worksheet XML")
}

fn replace_zip_entry(path: &std::path::Path, name: &str, replace: impl FnOnce(String) -> String) {
    let source = fs::read(path).expect("fixture bytes");
    let mut archive = ZipArchive::new(Cursor::new(source)).expect("open ZIP");
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    let mut replace = Some(replace);

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("ZIP entry");
        let entry_name = entry.name().to_owned();
        if entry_name == name {
            let mut xml = String::new();
            entry.read_to_string(&mut xml).expect("read worksheet XML");
            let replacement = replace.take().expect("replace exactly once")(xml);
            output
                .start_file(
                    entry_name,
                    SimpleFileOptions::default().compression_method(entry.compression()),
                )
                .expect("start replacement entry");
            output
                .write_all(replacement.as_bytes())
                .expect("write replacement entry");
        } else {
            output.raw_copy_file(entry).expect("copy ZIP entry");
        }
    }
    fs::write(path, output.finish().expect("finish ZIP").into_inner()).expect("write fixture");
}

fn append_zip_entry(path: &std::path::Path, name: &str, contents: &str) {
    let source = fs::read(path).expect("fixture bytes");
    let mut archive = ZipArchive::new(Cursor::new(source)).expect("open ZIP");
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let entry = archive.by_index(index).expect("ZIP entry");
        output.raw_copy_file(entry).expect("copy ZIP entry");
    }
    output
        .start_file(name, SimpleFileOptions::default())
        .expect("start additional entry");
    output
        .write_all(contents.as_bytes())
        .expect("write additional entry");
    fs::write(path, output.finish().expect("finish ZIP").into_inner()).expect("write fixture");
}
