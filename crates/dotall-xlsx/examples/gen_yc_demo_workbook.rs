//! Generate `demo/saas_model.xlsx` for the YC demo video.
//!
//! Formulas include cached results so LibreOffice/Excel open with real numbers
//! (not zeros) before recalculation.
//!
//! ```bash
//! cargo run -p dotall-xlsx --example gen_yc_demo_workbook
//! ```

use rust_xlsxwriter::{
    Chart, ChartType, Color, Format, FormatAlign, FormatBorder, Formula, Workbook, Worksheet,
    XlsxError,
};

const REGIONS: [&str; 8] = [
    "Americas", "EMEA", "APAC", "LATAM", "ANZ", "India", "Japan", "Canada",
];

/// Deterministic Jan seed units (Regions!B2:B9). Demo bump = ×1.25.
const JAN_SEEDS: [f64; 8] = [1200.0, 960.0, 800.0, 400.0, 320.0, 560.0, 640.0, 280.0];

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const LIST_PRICE: f64 = 49.0;
const MOM_GROWTH: f64 = 0.08;
const CHURN: f64 = 0.02;
const COGS_PCT: f64 = 0.22;
const MONTHLY_OPEX: f64 = 85_000.0;
const TAX_RATE: f64 = 0.21;

struct Model {
    /// regions[r][month 0..11]
    regions: [[f64; 12]; 8],
    revenue: [[f64; 12]; 8],
    pnl_revenue: [f64; 12],
    pnl_cogs: [f64; 12],
    pnl_gross: [f64; 12],
    pnl_opex: [f64; 12],
    pnl_ebt: [f64; 12],
    pnl_tax: [f64; 12],
    pnl_net: [f64; 12],
    fy_revenue: f64,
    fy_cogs: f64,
    fy_gross: f64,
    fy_opex: f64,
    fy_ebt: f64,
    fy_tax: f64,
    fy_net: f64,
}

impl Model {
    fn compute() -> Self {
        let mut regions = [[0.0; 12]; 8];
        let mut revenue = [[0.0; 12]; 8];
        for (r, seed) in JAN_SEEDS.iter().enumerate() {
            regions[r][0] = *seed;
            for m in 1..12 {
                regions[r][m] = regions[r][m - 1] * (1.0 + MOM_GROWTH);
            }
            for m in 0..12 {
                revenue[r][m] = regions[r][m] * LIST_PRICE;
            }
        }

        let mut pnl_revenue = [0.0; 12];
        let mut pnl_cogs = [0.0; 12];
        let mut pnl_gross = [0.0; 12];
        let mut pnl_opex = [0.0; 12];
        let mut pnl_ebt = [0.0; 12];
        let mut pnl_tax = [0.0; 12];
        let mut pnl_net = [0.0; 12];

        for m in 0..12 {
            pnl_revenue[m] = (0..8).map(|r| revenue[r][m]).sum();
            pnl_cogs[m] = pnl_revenue[m] * COGS_PCT;
            pnl_gross[m] = pnl_revenue[m] - pnl_cogs[m];
            pnl_opex[m] = MONTHLY_OPEX;
            pnl_ebt[m] = pnl_gross[m] - pnl_opex[m];
            pnl_tax[m] = pnl_ebt[m].max(0.0) * TAX_RATE;
            pnl_net[m] = pnl_ebt[m] - pnl_tax[m];
        }

        let fy_revenue = pnl_revenue.iter().sum();
        let fy_cogs = pnl_cogs.iter().sum();
        let fy_gross = pnl_gross.iter().sum();
        let fy_opex = pnl_opex.iter().sum();
        let fy_ebt = pnl_ebt.iter().sum();
        let fy_tax = pnl_tax.iter().sum();
        let fy_net = pnl_net.iter().sum();

        Self {
            regions,
            revenue,
            pnl_revenue,
            pnl_cogs,
            pnl_gross,
            pnl_opex,
            pnl_ebt,
            pnl_tax,
            pnl_net,
            fy_revenue,
            fy_cogs,
            fy_gross,
            fy_opex,
            fy_ebt,
            fy_tax,
            fy_net,
        }
    }
}

fn main() -> Result<(), XlsxError> {
    let out = output_path();
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create demo dir");
    }

    let model = Model::compute();
    let mut workbook = Workbook::new();

    let header = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0x1F4E79))
        .set_font_color(Color::White)
        .set_align(FormatAlign::Center);
    let label = Format::new().set_bold();
    let currency = Format::new().set_num_format("$#,##0.00");
    let currency_bold = Format::new().set_num_format("$#,##0.00").set_bold();
    let percent = Format::new().set_num_format("0.00%");
    let number = Format::new().set_num_format("#,##0");
    let section = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xD6DCE4));
    let _thin = Format::new().set_border(FormatBorder::Thin);

    build_assumptions(&mut workbook, &header, &label, &currency, &percent)?;
    build_regions(&mut workbook, &header, &number, &model)?;
    build_revenue(&mut workbook, &header, &currency, &model)?;
    build_pnl(
        &mut workbook,
        &header,
        &label,
        &currency,
        &currency_bold,
        &section,
        &model,
    )?;
    build_board(
        &mut workbook,
        &header,
        &label,
        &currency_bold,
        &percent,
        &model,
    )?;

    workbook.save(&out)?;
    println!("wrote {}", out.display());
    println!(
        "Jan seeds (Regions!B2:B9): {:?}",
        JAN_SEEDS.iter().map(|v| *v as i64).collect::<Vec<_>>()
    );
    println!(
        "Demo bump ×1.25 (Regions!B2:B9 after): {:?}",
        JAN_SEEDS
            .iter()
            .map(|v| (v * 1.25) as i64)
            .collect::<Vec<_>>()
    );
    println!(
        "Board ARR (cached) = ${:.2}  FY Revenue = ${:.2}",
        model.pnl_revenue[11] * 12.0,
        model.fy_revenue
    );
    Ok(())
}

fn output_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demo/saas_model.xlsx")
}

fn write_formula_result(
    sheet: &mut Worksheet,
    row: u32,
    col: u16,
    formula: &str,
    result: f64,
    format: &Format,
) -> Result<(), XlsxError> {
    let formula = Formula::new(formula).set_result(format_result(result));
    sheet.write_formula_with_format(row, col, formula, format)?;
    Ok(())
}

fn format_result(value: f64) -> String {
    // Excel cached results are plain decimal strings.
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

fn build_assumptions(
    workbook: &mut Workbook,
    header: &Format,
    label: &Format,
    currency: &Format,
    percent: &Format,
) -> Result<(), XlsxError> {
    let sheet = workbook.add_worksheet();
    sheet.set_name("Assumptions")?;
    sheet.set_tab_color(Color::RGB(0x1F4E79));
    sheet.write_string_with_format(0, 0, "Driver", header)?;
    sheet.write_string_with_format(0, 1, "Value", header)?;
    sheet.set_column_width(0, 18)?;
    sheet.set_column_width(1, 16)?;

    let rows: [(&str, AssumptionValue); 11] = [
        ("ListPrice", AssumptionValue::Currency(LIST_PRICE)),
        ("MoMGrowth", AssumptionValue::Percent(MOM_GROWTH)),
        ("Churn", AssumptionValue::Percent(CHURN)),
        ("COGS%", AssumptionValue::Percent(COGS_PCT)),
        ("MonthlyOpEx", AssumptionValue::Currency(MONTHLY_OPEX)),
        ("TaxRate", AssumptionValue::Percent(TAX_RATE)),
        ("SeatsPerAcct", AssumptionValue::Number(12.0)),
        ("Discount", AssumptionValue::Percent(0.10)),
        ("SupportCost", AssumptionValue::Currency(4.50)),
        ("MarketingPct", AssumptionValue::Percent(0.12)),
        ("TargetMargin", AssumptionValue::Percent(0.55)),
    ];

    for (idx, (name, value)) in rows.iter().enumerate() {
        let row = (idx + 1) as u32;
        sheet.write_string_with_format(row, 0, *name, label)?;
        match value {
            AssumptionValue::Currency(v) => {
                sheet.write_number_with_format(row, 1, *v, currency)?;
            }
            AssumptionValue::Percent(v) => {
                sheet.write_number_with_format(row, 1, *v, percent)?;
            }
            AssumptionValue::Number(v) => {
                sheet.write_number(row, 1, *v)?;
            }
        }
    }

    sheet.write_string(13, 0, "Acme SaaS FY2026 — edit B2:B5 in the YC demo")?;
    Ok(())
}

enum AssumptionValue {
    Currency(f64),
    Percent(f64),
    Number(f64),
}

fn build_regions(
    workbook: &mut Workbook,
    header: &Format,
    number: &Format,
    model: &Model,
) -> Result<(), XlsxError> {
    let sheet = workbook.add_worksheet();
    sheet.set_name("Regions")?;
    sheet.set_tab_color(Color::RGB(0x2E75B6));
    sheet.set_freeze_panes(1, 1)?;

    sheet.write_string_with_format(0, 0, "Region", header)?;
    sheet.set_column_width(0, 14)?;
    for (m, name) in MONTHS.iter().enumerate() {
        sheet.write_string_with_format(0, (m + 1) as u16, *name, header)?;
        sheet.set_column_width((m + 1) as u16, 14)?;
    }

    for (r, region) in REGIONS.iter().enumerate() {
        // Worksheet API is 0-based; A1 formula addresses are 1-based Excel rows.
        let row = (r + 1) as u32;
        let excel_row = row + 1;
        sheet.write_string(row, 0, *region)?;
        sheet.write_number_with_format(row, 1, model.regions[r][0], number)?;
        for month_col in 2..=12u16 {
            let prev = col_letter(month_col - 1);
            let formula = format!("={prev}{excel_row}*(1+Assumptions!$B$3)");
            let result = model.regions[r][(month_col - 1) as usize];
            write_formula_result(sheet, row, month_col, &formula, result, number)?;
        }
    }
    Ok(())
}

fn build_revenue(
    workbook: &mut Workbook,
    header: &Format,
    currency: &Format,
    model: &Model,
) -> Result<(), XlsxError> {
    let sheet = workbook.add_worksheet();
    sheet.set_name("Revenue")?;
    sheet.set_tab_color(Color::RGB(0x548235));
    sheet.set_freeze_panes(1, 1)?;

    sheet.write_string_with_format(0, 0, "Region", header)?;
    sheet.set_column_width(0, 14)?;
    for (m, name) in MONTHS.iter().enumerate() {
        sheet.write_string_with_format(0, (m + 1) as u16, *name, header)?;
        sheet.set_column_width((m + 1) as u16, 14)?;
    }

    for (r, region) in REGIONS.iter().enumerate() {
        let row = (r + 1) as u32;
        let excel_row = row + 1;
        sheet.write_string(row, 0, *region)?;
        for month_col in 1..=12u16 {
            let col = col_letter(month_col);
            let formula = format!("=Regions!${col}${excel_row}*Assumptions!$B$2");
            let result = model.revenue[r][(month_col - 1) as usize];
            write_formula_result(sheet, row, month_col, &formula, result, currency)?;
        }
    }
    Ok(())
}

fn build_pnl(
    workbook: &mut Workbook,
    header: &Format,
    label: &Format,
    currency: &Format,
    currency_bold: &Format,
    section: &Format,
    model: &Model,
) -> Result<(), XlsxError> {
    let sheet = workbook.add_worksheet();
    sheet.set_name("Income")?;
    sheet.set_tab_color(Color::RGB(0xC65911));

    sheet.write_string_with_format(0, 0, "Line Item", header)?;
    sheet.set_column_width(0, 16)?;
    for (m, name) in MONTHS.iter().enumerate() {
        sheet.write_string_with_format(0, (m + 1) as u16, *name, header)?;
        sheet.set_column_width((m + 1) as u16, 14)?;
    }
    sheet.write_string_with_format(0, 13, "FY Total", header)?;
    sheet.set_column_width(13, 16)?;

    sheet.write_string_with_format(1, 0, "Revenue", section)?;
    for month_col in 1..=12u16 {
        let col = col_letter(month_col);
        // Absolute refs so Excel/SharePoint don't rewrite column letters on load.
        let formula = format!("=SUM(Revenue!${col}$2:${col}$9)");
        write_formula_result(
            sheet,
            1,
            month_col,
            &formula,
            model.pnl_revenue[(month_col - 1) as usize],
            currency,
        )?;
    }
    write_formula_result(
        sheet,
        1,
        13,
        "=SUM($B$2:$M$2)",
        model.fy_revenue,
        currency_bold,
    )?;

    sheet.write_string_with_format(2, 0, "COGS", label)?;
    for month_col in 1..=12u16 {
        let col = col_letter(month_col);
        let formula = format!("=${col}$2*Assumptions!$B$5");
        write_formula_result(
            sheet,
            2,
            month_col,
            &formula,
            model.pnl_cogs[(month_col - 1) as usize],
            currency,
        )?;
    }
    write_formula_result(sheet, 2, 13, "=SUM($B$3:$M$3)", model.fy_cogs, currency)?;

    sheet.write_string_with_format(3, 0, "Gross Profit", section)?;
    for month_col in 1..=12u16 {
        let col = col_letter(month_col);
        let formula = format!("=${col}$2-${col}$3");
        write_formula_result(
            sheet,
            3,
            month_col,
            &formula,
            model.pnl_gross[(month_col - 1) as usize],
            currency,
        )?;
    }
    write_formula_result(
        sheet,
        3,
        13,
        "=SUM($B$4:$M$4)",
        model.fy_gross,
        currency_bold,
    )?;

    sheet.write_string_with_format(4, 0, "OpEx", label)?;
    for month_col in 1..=12u16 {
        write_formula_result(
            sheet,
            4,
            month_col,
            "=Assumptions!$B$6",
            model.pnl_opex[(month_col - 1) as usize],
            currency,
        )?;
    }
    write_formula_result(sheet, 4, 13, "=SUM($B$5:$M$5)", model.fy_opex, currency)?;

    sheet.write_string_with_format(5, 0, "EBT", label)?;
    for month_col in 1..=12u16 {
        let col = col_letter(month_col);
        let formula = format!("=${col}$4-${col}$5");
        write_formula_result(
            sheet,
            5,
            month_col,
            &formula,
            model.pnl_ebt[(month_col - 1) as usize],
            currency,
        )?;
    }
    write_formula_result(sheet, 5, 13, "=SUM($B$6:$M$6)", model.fy_ebt, currency)?;

    sheet.write_string_with_format(6, 0, "Tax", label)?;
    for month_col in 1..=12u16 {
        let col = col_letter(month_col);
        let formula = format!("=MAX(0,${col}$6)*Assumptions!$B$7");
        write_formula_result(
            sheet,
            6,
            month_col,
            &formula,
            model.pnl_tax[(month_col - 1) as usize],
            currency,
        )?;
    }
    write_formula_result(sheet, 6, 13, "=SUM($B$7:$M$7)", model.fy_tax, currency)?;

    sheet.write_string_with_format(7, 0, "Net Income", section)?;
    for month_col in 1..=12u16 {
        let col = col_letter(month_col);
        let formula = format!("=${col}$6-${col}$7");
        write_formula_result(
            sheet,
            7,
            month_col,
            &formula,
            model.pnl_net[(month_col - 1) as usize],
            currency_bold,
        )?;
    }
    write_formula_result(sheet, 7, 13, "=SUM($B$8:$M$8)", model.fy_net, currency_bold)?;

    Ok(())
}

fn build_board(
    workbook: &mut Workbook,
    header: &Format,
    label: &Format,
    currency_bold: &Format,
    percent: &Format,
    model: &Model,
) -> Result<(), XlsxError> {
    let sheet = workbook.add_worksheet();
    sheet.set_name("Board")?;
    sheet.set_tab_color(Color::RGB(0x833C0C));

    sheet.write_string_with_format(0, 0, "KPI", header)?;
    sheet.write_string_with_format(0, 1, "Value", header)?;
    sheet.set_column_width(0, 18)?;
    sheet.set_column_width(1, 18)?;
    for col in 2u16..=12 {
        sheet.set_column_width(col, 14)?;
    }

    let arr = model.pnl_revenue[11] * 12.0;
    let mrr = model.pnl_revenue[11];
    let gross_margin = if model.fy_revenue == 0.0 {
        0.0
    } else {
        model.fy_gross / model.fy_revenue
    };

    sheet.write_string_with_format(1, 0, "ARR", label)?;
    write_formula_result(sheet, 1, 1, "=Income!$M$2*12", arr, currency_bold)?;

    sheet.write_string_with_format(2, 0, "MRR", label)?;
    write_formula_result(sheet, 2, 1, "=Income!$M$2", mrr, currency_bold)?;

    sheet.write_string_with_format(3, 0, "FY Revenue", label)?;
    write_formula_result(sheet, 3, 1, "=Income!$N$2", model.fy_revenue, currency_bold)?;

    sheet.write_string_with_format(4, 0, "Gross Margin", label)?;
    write_formula_result(
        sheet,
        4,
        1,
        "=IF(Income!$N$2=0,0,Income!$N$4/Income!$N$2)",
        gross_margin,
        percent,
    )?;

    sheet.write_string_with_format(5, 0, "FY Net Income", label)?;
    write_formula_result(sheet, 5, 1, "=Income!$N$8", model.fy_net, currency_bold)?;

    sheet.write_string_with_format(6, 0, "List Price", label)?;
    write_formula_result(sheet, 6, 1, "=Assumptions!$B$2", LIST_PRICE, currency_bold)?;

    // Chart source below the chart; absolute Income refs so every month column works.
    const CHART_CAT_ROW: u32 = 24; // Excel row 25
    const CHART_VAL_ROW: u32 = 25; // Excel row 26
    sheet.write_string_with_format(CHART_CAT_ROW, 0, "Month", header)?;
    sheet.write_string_with_format(CHART_VAL_ROW, 0, "Revenue", header)?;
    for (m, name) in MONTHS.iter().enumerate() {
        let col = (m + 1) as u16;
        let income_col = col_letter(col);
        sheet.write_string(CHART_CAT_ROW, col, *name)?;
        write_formula_result(
            sheet,
            CHART_VAL_ROW,
            col,
            &format!("=Income!${income_col}$2"),
            model.pnl_revenue[m],
            currency_bold,
        )?;
    }

    let mut chart = Chart::new(ChartType::Column);
    chart.set_width(640);
    chart.set_height(260);
    chart
        .add_series()
        .set_categories("Board!$B$25:$M$25")
        .set_values("Board!$B$26:$M$26")
        .set_name("Monthly Revenue");
    chart.title().set_name("FY2026 Monthly Revenue");
    sheet.insert_chart(1, 3, &chart)?;

    sheet.write_string(
        27,
        0,
        "Board pack — chart is preserve-only in the Dotall demo",
    )?;
    Ok(())
}

fn col_letter(col: u16) -> char {
    char::from(b'A' + (col as u8))
}
