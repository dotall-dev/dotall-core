use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
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
fn stage_edit_leaves_source_unchanged() {
    let temp = tempdir().expect("tempdir");
    let workbook = temp.path().join("book.xlsx");
    workbook_fixture(&workbook);
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = workbook.to_str().expect("UTF-8 workbook");
    let before = fs::read(&workbook).expect("source bytes");

    dotall().args(["init", workspace]).assert().success();
    dotall()
        .args([
            "edit",
            source,
            "--op",
            "set_cell_value",
            "--sheet",
            "Sheet",
            "--address",
            "B2",
            "--value",
            "99",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("staged edit"));

    assert_eq!(fs::read(&workbook).expect("source after stage"), before);
}

#[test]
fn apply_updates_source_history_and_diff() {
    let temp = tempdir().expect("tempdir");
    let workbook = temp.path().join("book.xlsx");
    workbook_fixture(&workbook);
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = workbook.to_str().expect("UTF-8 workbook");

    dotall().args(["init", workspace]).assert().success();

    let staged = dotall()
        .args([
            "--json",
            "edit",
            source,
            "--op",
            "set_cell_value",
            "--sheet",
            "Sheet",
            "--address",
            "B2",
            "--value",
            "99",
        ])
        .output()
        .expect("stage edit");
    assert!(staged.status.success());
    let staged: serde_json::Value =
        serde_json::from_slice(&staged.stdout).expect("stage JSON output");
    let tx_id = staged["tx_id"].as_str().expect("tx_id");

    dotall()
        .args(["apply", source, "--tx", tx_id])
        .assert()
        .success();

    dotall()
        .args(["read", source, "--range", "Sheet!A1:B2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("| Alpha | 99 |"));

    let history = dotall()
        .args(["--json", "history", source])
        .output()
        .expect("history output");
    assert!(history.status.success());
    let history: serde_json::Value =
        serde_json::from_slice(&history.stdout).expect("history JSON output");
    assert_eq!(history["entries"].as_array().expect("entries").len(), 1);
    assert_eq!(history["entries"][0]["version"], 1);

    let diff = dotall()
        .args(["--json", "diff", source, "--version", "1"])
        .output()
        .expect("diff output");
    assert!(diff.status.success());
    let diff: serde_json::Value = serde_json::from_slice(&diff.stdout).expect("diff JSON output");
    assert_eq!(diff["version"], 1);
    assert_eq!(diff["semantic_diff"][0]["target"], "Sheet!B2");
    assert_eq!(diff["semantic_diff"][0]["after"], "99");
}

#[test]
fn revert_stages_restore_and_apply_commits_new_version() {
    let temp = tempdir().expect("tempdir");
    let workbook = temp.path().join("book.xlsx");
    workbook_fixture(&workbook);
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = workbook.to_str().expect("UTF-8 workbook");

    dotall().args(["init", workspace]).assert().success();

    let staged = dotall()
        .args([
            "--json",
            "edit",
            source,
            "--op",
            "set_cell_value",
            "--sheet",
            "Sheet",
            "--address",
            "B2",
            "--value",
            "99",
        ])
        .output()
        .expect("stage edit");
    assert!(staged.status.success());
    let staged: serde_json::Value =
        serde_json::from_slice(&staged.stdout).expect("stage JSON output");
    let tx_id = staged["tx_id"].as_str().expect("tx_id");

    dotall()
        .args(["apply", source, "--tx", tx_id])
        .assert()
        .success();

    let revert = dotall()
        .args(["--json", "revert", source, "--version", "1"])
        .output()
        .expect("revert output");
    assert!(revert.status.success());
    let revert: serde_json::Value =
        serde_json::from_slice(&revert.stdout).expect("revert JSON output");
    let revert_tx = revert["tx_id"].as_str().expect("revert tx_id");

    dotall()
        .args(["read", source, "--range", "Sheet!A1:B2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("| Alpha | 99 |"));

    dotall()
        .args(["apply", source, "--tx", revert_tx])
        .assert()
        .success();

    dotall()
        .args(["read", source, "--range", "Sheet!A1:B2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("| Alpha | 42 |"));

    let history = dotall()
        .args(["--json", "history", source])
        .output()
        .expect("history output");
    assert!(history.status.success());
    let history: serde_json::Value =
        serde_json::from_slice(&history.stdout).expect("history JSON output");
    assert_eq!(history["entries"].as_array().expect("entries").len(), 2);
    assert_eq!(history["entries"][1]["version"], 2);
}

#[test]
fn staged_and_discard_manage_pending_edits() {
    let temp = tempdir().expect("tempdir");
    let workbook = temp.path().join("book.xlsx");
    workbook_fixture(&workbook);
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = workbook.to_str().expect("UTF-8 workbook");

    dotall().args(["init", workspace]).assert().success();

    let staged = dotall()
        .args([
            "--json",
            "edit",
            source,
            "--op",
            "set_cell_value",
            "--sheet",
            "Sheet",
            "--address",
            "B2",
            "--value",
            "99",
        ])
        .output()
        .expect("stage edit");
    assert!(staged.status.success());
    let staged: serde_json::Value =
        serde_json::from_slice(&staged.stdout).expect("stage JSON output");
    let tx_id = staged["tx_id"].as_str().expect("tx_id");

    let pending = dotall()
        .args(["--json", "staged", source])
        .output()
        .expect("staged output");
    assert!(pending.status.success());
    let pending: serde_json::Value =
        serde_json::from_slice(&pending.stdout).expect("staged JSON output");
    assert_eq!(pending["edits"].as_array().expect("edits").len(), 1);
    assert_eq!(pending["edits"][0]["tx_id"], tx_id);

    dotall()
        .args(["discard", source, "--tx", tx_id])
        .assert()
        .success();

    let pending = dotall()
        .args(["--json", "staged", source])
        .output()
        .expect("staged output");
    assert!(pending.status.success());
    let pending: serde_json::Value =
        serde_json::from_slice(&pending.stdout).expect("staged JSON output");
    assert_eq!(pending["edits"].as_array().expect("edits").len(), 0);
}
