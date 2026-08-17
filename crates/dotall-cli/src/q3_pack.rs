//! Shared Q3 board-pack demo generators used by `generate_demos` and tests.

use std::fs;
use std::path::Path;

use rust_xlsxwriter::{DocProperties, ExcelDateTime, Format, Workbook};

/// Fixed creation datetime so regenerated Q3 financials stay byte-stable.
fn pin_workbook_datetime(workbook: &mut Workbook) -> Result<(), Box<dyn std::error::Error>> {
    let date = ExcelDateTime::from_ymd(2026, 8, 17)?;
    let properties = DocProperties::new().set_creation_datetime(&date);
    workbook.set_properties(&properties);
    Ok(())
}

/// Write the heavy Q3 pack into `dir`:
/// `q3-financials.xlsx`, `q3-deck.pptx`, `q3-memo.docx`, `q3-intake.pdf`.
pub fn write_q3_pack(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(dir)?;
    write_q3_financials(&dir.join("q3-financials.xlsx"))?;
    fs::write(dir.join("q3-deck.pptx"), dotall_pptx::demo_q3_deck_pptx())?;
    fs::write(dir.join("q3-memo.docx"), dotall_docx::demo_q3_memo_docx())?;
    fs::write(dir.join("q3-intake.pdf"), dotall_pdf::demo_q3_intake_pdf())?;
    Ok(())
}

fn write_q3_financials(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut workbook = Workbook::new();
    pin_workbook_datetime(&mut workbook)?;
    let header = Format::new().set_bold();
    let percent = Format::new().set_num_format("0%");
    let currency = Format::new().set_num_format("$#,##0.00");

    let inputs = workbook.add_worksheet().set_name("Inputs")?;
    inputs.set_column_width(0, 14.0)?;
    inputs.set_column_width(1, 12.0)?;
    inputs.write_string(1, 0, "Rate")?;
    inputs.write_number_with_format(1, 1, 0.10, &percent)?;
    inputs.write_string(2, 0, "Base")?;
    inputs.write_number_with_format(2, 1, 100.0, &currency)?;

    let revenue = workbook.add_worksheet().set_name("Revenue")?;
    revenue.set_column_width(0, 14.0)?;
    revenue.set_column_width(1, 14.0)?;
    revenue.write_string_with_format(0, 0, "Label", &header)?;
    revenue.write_string_with_format(0, 1, "Amount", &header)?;
    revenue.write_string(1, 0, "Jan")?;
    revenue.write_number_with_format(1, 1, 100.0, &currency)?;
    revenue.write_string(2, 0, "Feb")?;
    revenue.write_number_with_format(2, 1, 150.0, &currency)?;
    revenue.write_string(3, 0, "Total")?;
    revenue.write_formula_with_format(3, 1, "=B2+B3", &currency)?;
    revenue.write_string(4, 0, "Commission")?;
    revenue.write_formula_with_format(4, 1, "=B4*Inputs!B2", &currency)?;

    for month in 1..=12u8 {
        let name = format!("FY2024-{month:02}");
        let sheet = workbook.add_worksheet().set_name(&name)?;
        sheet.write_string_with_format(0, 0, "Row", &header)?;
        sheet.write_string_with_format(0, 1, "Note", &header)?;
        sheet.write_string_with_format(0, 2, "Amount", &header)?;
        for row in 1..=80u32 {
            let excel_row = row; // 0-based: row 1..80 => Excel rows 2..81
            sheet.write_number(excel_row, 0, f64::from(row))?;
            if row == 1 {
                sheet.write_string(excel_row, 1, "legacy 10% promo")?;
            } else {
                sheet.write_string(excel_row, 1, format!("history line {row}"))?;
            }
            sheet.write_number_with_format(excel_row, 2, f64::from(row) * 10.0, &currency)?;
        }
    }

    workbook.define_name("Rate", "=Inputs!$B$2")?;
    workbook.save(path)?;
    Ok(())
}
