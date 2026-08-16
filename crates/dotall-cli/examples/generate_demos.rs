//! Regenerate committed files under `demo/`.
//!
//! ```bash
//! cargo run -p dotall-cli --example generate_demos
//! ```

use std::fs;
use std::path::PathBuf;

use rust_xlsxwriter::{Format, Workbook};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let demo = demo_dir()?;
    fs::create_dir_all(&demo)?;

    write_financials(&demo.join("financials.xlsx"))?;
    fs::write(demo.join("deck.pptx"), dotall_pptx::demo_deck_pptx())?;
    fs::write(demo.join("memo.docx"), dotall_docx::demo_memo_docx())?;
    fs::write(demo.join("form.pdf"), dotall_pdf::demo_form_pdf())?;

    println!("Wrote demo files to {}", demo.display());
    Ok(())
}

fn demo_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(manifest
        .parent()
        .and_then(|p| p.parent())
        .ok_or("workspace root")?
        .join("demo"))
}

fn write_financials(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let mut workbook = Workbook::new();
    let header = Format::new().set_bold();
    let percent = Format::new().set_num_format("0%");
    let currency = Format::new().set_num_format("$#,##0.00");

    let inputs = workbook.add_worksheet().set_name("Inputs")?;
    inputs.set_column_width(0, 14.0)?;
    inputs.set_column_width(1, 12.0)?;
    inputs.set_row_height(0, 22.0)?;
    inputs.merge_range(0, 0, 0, 1, "Assumptions", &header)?;
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

    workbook.define_name("Rate", "=Inputs!$B$2")?;
    workbook.save(path)?;
    Ok(())
}
