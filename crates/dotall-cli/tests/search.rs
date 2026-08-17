use assert_cmd::Command;
use predicates::prelude::*;
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

fn dotall() -> Command {
    Command::cargo_bin("dotall").expect("binary")
}

fn rate_workbook(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let sheet = workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("sheet name");
    sheet.write_string(0, 0, "Rate").expect("header");
    sheet.write_number(1, 0, 0.0825).expect("value");
    workbook
        .define_name("Rate", "=Inputs!$A$2")
        .expect("defined name");
    workbook.save(path).expect("workbook fixture");
}

#[test]
fn search_json_returns_named_range_hit_after_inspect() {
    let temp = tempdir().expect("tempdir");
    let workbook = temp.path().join("book.xlsx");
    rate_workbook(&workbook);
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = workbook.to_str().expect("UTF-8 workbook");

    dotall().args(["init", workspace]).assert().success();
    dotall().args(["inspect", source]).assert().success();

    let output = dotall()
        .args(["--json", "search", "Rate", workspace])
        .output()
        .expect("search output");

    assert!(output.status.success(), "search should succeed");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");

    let hits = value["hits"].as_array().expect("hits array");
    assert!(
        !hits.is_empty(),
        "expected at least one hit for `Rate`, got: {value}"
    );
    assert!(
        hits.iter().any(|hit| {
            hit["selector_kind"].as_str() == Some("named_ranges")
                && hit["selector"].as_str() == Some("Rate")
        }),
        "expected a named_ranges hit for `Rate`, got: {value}"
    );
    assert!(
        value["not_indexed"].as_array().is_none_or(|v| v.is_empty()),
        "expected no not_indexed entries after inspect, got: {value}"
    );
}

#[test]
fn search_text_prints_one_hit_per_line() {
    let temp = tempdir().expect("tempdir");
    let workbook = temp.path().join("book.xlsx");
    rate_workbook(&workbook);
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = workbook.to_str().expect("UTF-8 workbook");

    dotall().args(["init", workspace]).assert().success();
    dotall().args(["inspect", source]).assert().success();

    let output = dotall()
        .args(["search", "Rate", workspace])
        .output()
        .expect("search output");

    assert!(output.status.success(), "search should succeed");
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let first_line = stdout.lines().next().expect("at least one hit line");
    assert!(
        first_line.contains("book.xlsx"),
        "expected path in first line, got: {first_line}"
    );
    assert!(
        first_line.contains("named_ranges"),
        "expected selector_kind in line, got: {first_line}"
    );
    assert!(
        first_line.contains("Rate"),
        "expected selector/snippet in line, got: {first_line}"
    );
}

#[test]
fn search_empty_query_fails_with_query_in_stderr() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    dotall().args(["init", workspace]).assert().success();

    dotall()
        .args(["search", "   ", workspace])
        .assert()
        .failure()
        .stderr(predicate::str::contains("query"));
}

#[test]
fn search_glob_filters_out_non_matching_paths() {
    let temp = tempdir().expect("tempdir");
    let workbook = temp.path().join("book.xlsx");
    rate_workbook(&workbook);
    let other_path = temp.path().join("other.xlsx");
    rate_workbook(&other_path);
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let book_source = workbook.to_str().expect("UTF-8 workbook");
    let other_source = other_path.to_str().expect("UTF-8 other workbook");

    dotall().args(["init", workspace]).assert().success();
    dotall().args(["inspect", book_source]).assert().success();
    dotall().args(["inspect", other_source]).assert().success();

    let output = dotall()
        .args(["--json", "search", "Rate", "--glob", "book.xlsx", workspace])
        .output()
        .expect("search output");

    assert!(output.status.success(), "search should succeed");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    let hits = value["hits"].as_array().expect("hits array");
    assert!(
        hits.iter()
            .all(|hit| hit["path"].as_str() == Some("book.xlsx")),
        "glob should filter to book.xlsx only, got: {value}"
    );
    assert!(
        !hits.is_empty(),
        "expected hits for book.xlsx, got: {value}"
    );
}
