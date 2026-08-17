use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn init_creates_project_storage() {
    let temp = tempdir().expect("tempdir");

    Command::cargo_bin("dotall")
        .expect("binary")
        .args(["init", temp.path().to_str().expect("UTF-8 path")])
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized Dotall"));

    assert!(temp.path().join(".all/manifest.json").is_file());
}

#[test]
fn json_status_is_machine_readable() {
    let temp = tempdir().expect("tempdir");
    Command::cargo_bin("dotall")
        .expect("binary")
        .args(["init", temp.path().to_str().expect("UTF-8 path")])
        .assert()
        .success();

    let output = Command::cargo_bin("dotall")
        .expect("binary")
        .args([
            "--json",
            "status",
            temp.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("status output");

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(value["tracked_count"], 0);
    assert_eq!(value["objects"], serde_json::json!([]));
}

#[test]
fn status_outside_workspace_has_actionable_error() {
    let temp = tempdir().expect("tempdir");

    Command::cargo_bin("dotall")
        .expect("binary")
        .args(["status", temp.path().to_str().expect("UTF-8 path")])
        .assert()
        .failure()
        .stderr(predicate::str::contains("dotall init"));
}

#[test]
fn unknown_convenience_op_points_at_ops_json() {
    let temp = tempdir().expect("tempdir");
    Command::cargo_bin("dotall")
        .expect("binary")
        .args(["init", temp.path().to_str().expect("UTF-8 path")])
        .assert()
        .success();
    let file = temp.path().join("deck.pptx");
    std::fs::write(&file, b"not-a-real-pptx").expect("file");

    Command::cargo_bin("dotall")
        .expect("binary")
        .args([
            "edit",
            file.to_str().expect("UTF-8"),
            "--op",
            "set_shape_text",
            "--sheet",
            "unused",
            "--address",
            "A1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--ops-json"))
        .stderr(predicate::str::contains("xlsx").not());
}
