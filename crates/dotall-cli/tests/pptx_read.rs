use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

fn dotall() -> Command {
    Command::cargo_bin("dotall").expect("binary")
}

#[test]
fn inspect_and_read_pptx_in_initialized_workspace() {
    let temp = tempdir().expect("tempdir");
    let deck = temp.path().join("deck.pptx");
    fs::write(&deck, dotall_pptx::minimal_pptx()).expect("write fixture");
    let workspace = temp.path().to_str().expect("UTF-8 workspace");
    let source = deck.to_str().expect("UTF-8 deck");

    dotall().args(["init", workspace]).assert().success();

    dotall()
        .args(["inspect", source])
        .assert()
        .success()
        .stdout(predicates::str::contains("Slide 1"));

    dotall()
        .args([
            "read",
            source,
            "--selector-kind",
            "slide",
            "--selector",
            "Slide 1",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("Hello"))
        .stdout(predicates::str::contains("Title"));
}
