use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

fn dotall() -> Command {
    Command::cargo_bin("dotall").expect("binary")
}

#[test]
fn inspect_and_read_pdf_in_initialized_workspace() {
    let temp = tempdir().expect("tempdir");
    let form = temp.path().join("form.pdf");
    fs::write(&form, dotall_pdf::minimal_form_pdf()).expect("write fixture");
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = form.to_str().expect("UTF-8 form");

    dotall().args(["init", workspace]).assert().success();

    dotall()
        .args(["inspect", source])
        .assert()
        .success()
        .stdout(predicates::str::contains("Name"));

    dotall()
        .args([
            "read",
            source,
            "--selector-kind",
            "field",
            "--selector",
            "Name",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("Ada"));
}
