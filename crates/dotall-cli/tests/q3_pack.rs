use std::path::Path;

use dotall_cli::q3_pack::write_q3_pack;
use dotall_xlsx::{CellValue, parse_workbook};
use tempfile::tempdir;

#[test]
fn q3_pack_writes_heavy_workbook_and_siblings() {
    let temp = tempdir().expect("tempdir");
    let dir = temp.path().join("q3-pack");
    write_q3_pack(&dir).expect("write_q3_pack");

    assert!(dir.join("q3-financials.xlsx").is_file());
    assert!(dir.join("q3-deck.pptx").is_file());
    assert!(dir.join("q3-memo.docx").is_file());
    assert!(dir.join("q3-intake.pdf").is_file());

    assert_q3_financials(&dir.join("q3-financials.xlsx"));
}

fn assert_q3_financials(path: &Path) {
    let model = parse_workbook(path).expect("parse q3-financials");

    let sheet_names: Vec<_> = model.sheets.iter().map(|s| s.name.as_str()).collect();
    assert!(sheet_names.contains(&"Inputs"));
    assert!(sheet_names.contains(&"Revenue"));
    for month in 1..=12 {
        let name = format!("FY2024-{month:02}");
        assert!(sheet_names.contains(&name.as_str()), "missing sheet {name}");
    }

    let inputs = model
        .sheets
        .iter()
        .find(|s| s.name == "Inputs")
        .expect("Inputs");
    let b2 = inputs
        .cells
        .iter()
        .find(|c| c.address == "B2")
        .expect("Inputs!B2");
    match &b2.value {
        CellValue::Float(v) => assert!((v - 0.10).abs() < 1e-9, "B2={v}"),
        CellValue::Integer(v) => assert_eq!(*v, 0),
        other => panic!("Inputs!B2 unexpected value: {other:?}"),
    }
    let fmt = b2.number_format.as_deref().unwrap_or("");
    assert!(
        fmt.contains('%') || fmt.contains("0%"),
        "Inputs!B2 should be percent-formatted, got {fmt:?}"
    );

    let rate = model
        .named_ranges
        .iter()
        .find(|n| n.name == "Rate")
        .expect("named range Rate");
    assert!(
        rate.formula.contains("Inputs!$B$2"),
        "Rate formula={}",
        rate.formula
    );

    let revenue = model
        .sheets
        .iter()
        .find(|s| s.name == "Revenue")
        .expect("Revenue");
    let b5 = revenue
        .cells
        .iter()
        .find(|c| c.address == "B5")
        .expect("Revenue!B5");
    let formula = b5.formula.as_deref().unwrap_or("");
    assert!(
        formula.contains("B4") && formula.contains("Inputs!B2"),
        "Revenue!B5 formula={formula}"
    );

    let fy = model
        .sheets
        .iter()
        .find(|s| s.name == "FY2024-01")
        .expect("FY2024-01");
    assert!(
        fy.dimensions.rows >= 80,
        "FY2024-01 rows={}",
        fy.dimensions.rows
    );
    let decoy = fy.cells.iter().any(|c| match &c.value {
        CellValue::String(s) => s.contains("legacy 10% promo"),
        _ => false,
    });
    assert!(decoy, "FY history sheet missing decoy string");

    let inputs_has_decoy = inputs.cells.iter().any(|c| match &c.value {
        CellValue::String(s) => s.contains("legacy 10% promo"),
        _ => false,
    });
    assert!(
        !inputs_has_decoy,
        "Inputs must not carry the history-sheet decoy"
    );
}
