use std::fs;

use dotall_core::{DotallStore, ObjectState};
use tempfile::tempdir;

#[test]
fn init_is_idempotent_and_creates_required_layout() {
    let temp = tempdir().expect("tempdir");

    DotallStore::init(temp.path()).expect("first init");
    DotallStore::init(temp.path()).expect("second init");

    assert!(temp.path().join(".all/manifest.json").is_file());
    assert!(temp.path().join(".all/objects").is_dir());
}

#[test]
fn registered_source_moves_from_fresh_to_stale() {
    let temp = tempdir().expect("tempdir");
    let source = temp.path().join("book.xlsx");
    fs::write(&source, b"initial").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("init");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register");

    assert!(temp.path().join(".all/objects/book.xlsx/meta.json").is_file());
    assert!(temp.path().join(".all/objects/book.xlsx/original.ref").is_file());
    let fresh = store.status().expect("fresh status");
    assert_eq!(fresh[0].state, ObjectState::FreshFastPath);

    fs::write(&source, b"changed and larger").expect("change source");
    let stale = store.status().expect("stale status");
    assert_eq!(stale[0].state, ObjectState::Stale);
}

#[test]
fn status_reports_deleted_source_as_missing() {
    let temp = tempdir().expect("tempdir");
    let source = temp.path().join("book.xlsx");
    fs::write(&source, b"initial").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("init");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register");
    fs::remove_file(source).expect("remove source");

    let status = store.status().expect("status");

    assert_eq!(status[0].state, ObjectState::Missing);
}

#[test]
fn parent_traversal_cannot_escape_the_workspace() {
    let temp = tempdir().expect("tempdir");
    let mut store = DotallStore::init(temp.path()).expect("init");

    let error = store
        .register_source("../outside.xlsx", "xlsx")
        .expect_err("unsafe path");

    assert!(error.to_string().contains("invalid source path"));
}
