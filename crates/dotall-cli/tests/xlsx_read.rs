use std::fs;
use std::time::SystemTime;

use assert_cmd::Command;
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

fn workbook_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Sheet").expect("sheet name");
    worksheet.write_string(0, 0, "Name").expect("header");
    worksheet.write_string(0, 1, "Value").expect("header");
    worksheet.write_string(1, 0, "Alpha").expect("value");
    worksheet.write_number(1, 1, 42.0).expect("value");
    workbook.save(path).expect("workbook fixture");
}

fn dotall() -> Command {
    Command::cargo_bin("dotall").expect("binary")
}

#[test]
fn inspect_and_read_xlsx_in_initialized_workspace() {
    let temp = tempdir().expect("tempdir");
    let workbook = temp.path().join("book.xlsx");
    workbook_fixture(&workbook);
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = workbook.to_str().expect("UTF-8 workbook");

    dotall().args(["init", workspace]).assert().success();

    dotall()
        .args(["inspect", source])
        .assert()
        .success()
        .stdout(predicates::str::contains("Sheet"));

    dotall()
        .args(["read", source, "--range", "Sheet!A1:B2"])
        .assert()
        .success()
        .stdout(predicates::str::contains("| Name | Value |"))
        .stdout(predicates::str::contains("| Alpha | 42 |"));
}

#[test]
fn warm_inspect_reuses_the_cached_model_without_rewriting_it() {
    let temp = tempdir().expect("tempdir");
    let workbook = temp.path().join("book.xlsx");
    workbook_fixture(&workbook);
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = workbook.to_str().expect("UTF-8 workbook");

    dotall().args(["init", workspace]).assert().success();
    dotall().args(["inspect", source]).assert().success();
    let model = temp
        .path()
        .join(".all/objects/book.xlsx/cache/model/model.json");
    let first_modified = fs::metadata(&model)
        .expect("cached model metadata")
        .modified()
        .expect("cached model modification time");

    dotall().args(["inspect", source]).assert().success();
    let second_modified: SystemTime = fs::metadata(&model)
        .expect("cached model metadata")
        .modified()
        .expect("cached model modification time");

    assert_eq!(second_modified, first_modified);
}

#[test]
fn inspect_json_is_machine_readable() {
    let temp = tempdir().expect("tempdir");
    let workbook = temp.path().join("book.xlsx");
    workbook_fixture(&workbook);
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = workbook.to_str().expect("UTF-8 workbook");

    dotall().args(["init", workspace]).assert().success();
    let output = dotall()
        .args(["--json", "inspect", source])
        .output()
        .expect("inspect output");

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(value["inspection"]["format_id"], "xlsx");
    assert_eq!(value["inspection"]["summary"]["sheets"][0]["name"], "Sheet");
}
