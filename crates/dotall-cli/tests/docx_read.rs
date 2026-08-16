use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

fn dotall() -> Command {
    Command::cargo_bin("dotall").expect("binary")
}

#[test]
fn inspect_and_read_docx_in_initialized_workspace() {
    let temp = tempdir().expect("tempdir");
    let memo = temp.path().join("memo.docx");
    fs::write(&memo, dotall_docx::minimal_docx()).expect("write fixture");
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = memo.to_str().expect("UTF-8 memo");

    dotall().args(["init", workspace]).assert().success();

    dotall()
        .args(["inspect", source])
        .assert()
        .success()
        .stdout(predicates::str::contains("Alpha"));

    dotall()
        .args([
            "read",
            source,
            "--selector-kind",
            "paragraphs",
            "--selector",
            "0:2",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("Beta"));
}
