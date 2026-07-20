use std::fs;
use std::time::SystemTime;

use assert_cmd::Command;
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

fn workbook_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Revenue").expect("sheet name");
    worksheet.write_number(1, 1, 100.0).expect("source value");
    worksheet.write_number(2, 1, 50.0).expect("source value");
    worksheet
        .write_formula(3, 1, "=B2+B3")
        .expect("dependent formula");
    workbook.save(path).expect("workbook fixture");
}

fn dotall() -> Command {
    Command::cargo_bin("dotall").expect("binary")
}

#[test]
fn queries_formula_dependencies_and_reuses_the_derived_cache() {
    let temp = tempdir().expect("tempdir");
    let workbook = temp.path().join("book.xlsx");
    workbook_fixture(&workbook);
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = workbook.to_str().expect("UTF-8 workbook");

    dotall().args(["init", workspace]).assert().success();

    let forward = dotall()
        .args(["--json", "deps", source, "--cell", "Revenue!B4"])
        .output()
        .expect("forward dependencies");
    assert!(forward.status.success());
    let forward: serde_json::Value =
        serde_json::from_slice(&forward.stdout).expect("forward JSON output");
    assert_eq!(forward["direction"], "forward");
    assert_eq!(forward["edges"].as_array().expect("forward edges").len(), 2);

    let derived_dir = temp.path().join(".all/objects/book.xlsx/cache/derived");
    let derived = fs::read_dir(&derived_dir)
        .expect("derived cache directory")
        .next()
        .expect("derived cache entry")
        .expect("derived cache entry")
        .path();
    let first_modified = fs::metadata(&derived)
        .expect("derived cache metadata")
        .modified()
        .expect("derived cache modification time");

    let reverse = dotall()
        .args([
            "--json",
            "deps",
            source,
            "--cell",
            "Revenue!B2",
            "--dependents",
        ])
        .output()
        .expect("reverse dependencies");
    assert!(reverse.status.success());
    let reverse: serde_json::Value =
        serde_json::from_slice(&reverse.stdout).expect("reverse JSON output");
    assert_eq!(reverse["direction"], "reverse");
    assert_eq!(reverse["edges"].as_array().expect("reverse edges").len(), 1);

    let second_modified: SystemTime = fs::metadata(&derived)
        .expect("derived cache metadata")
        .modified()
        .expect("derived cache modification time");
    assert_eq!(second_modified, first_modified);
}
