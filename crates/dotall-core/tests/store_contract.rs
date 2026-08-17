use std::fs;
use std::time::{Duration, SystemTime};

use dotall_core::{DotallStore, ObjectState};
use filetime::{FileTime, set_file_mtime};
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
fn init_creates_missing_workspace_directory() {
    let temp = tempdir().expect("tempdir");
    let missing = temp.path().join("new-workspace");

    DotallStore::init(&missing).expect("init missing directory");

    assert!(missing.join(".all/manifest.json").is_file());
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

    assert!(
        temp.path()
            .join(".all/objects/book.xlsx/meta.json")
            .is_file()
    );
    assert!(
        temp.path()
            .join(".all/objects/book.xlsx/original.ref")
            .is_file()
    );
    let fresh = store.status().expect("fresh status");
    assert_eq!(fresh[0].state, ObjectState::FreshFastPath);

    fs::write(&source, b"changed and larger").expect("change source");
    let stale = store.status().expect("stale status");
    assert_eq!(stale[0].state, ObjectState::Stale);
}

#[test]
fn refresh_writes_back_fingerprint_after_mtime_only_change() {
    let temp = tempdir().expect("tempdir");
    let source = temp.path().join("book.xlsx");
    fs::write(&source, b"same-bytes").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("init");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register");
    let before = store.manifest().objects["book.xlsx"].fingerprint.clone();

    let later = SystemTime::now() + Duration::from_secs(5);
    set_file_mtime(&source, FileTime::from_system_time(later)).expect("mtime");

    let hashed = store.status().expect("status after mtime");
    assert_eq!(hashed[0].state, ObjectState::FreshAfterHash);
    assert_eq!(
        store.manifest().objects["book.xlsx"].fingerprint,
        before,
        "read-only status must not persist the new mtime"
    );

    let refreshed = store
        .refresh_fresh_fingerprints()
        .expect("write-back fingerprints");
    assert_eq!(refreshed[0].state, ObjectState::FreshAfterHash);
    let after = store.manifest().objects["book.xlsx"].fingerprint.clone();
    assert_eq!(after.blake3, before.blake3);
    assert_ne!(
        after.modified_unix_nanos, before.modified_unix_nanos,
        "mtime-only freshness should persist the new fingerprint"
    );

    let fast = store.status().expect("status after write-back");
    assert_eq!(fast[0].state, ObjectState::FreshFastPath);

    let reopened = DotallStore::open(temp.path()).expect("reopen");
    assert_eq!(
        reopened.status().expect("reopened status")[0].state,
        ObjectState::FreshFastPath
    );
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

#[test]
fn dotted_all_paths_cannot_register_inside_store() {
    let temp = tempdir().expect("tempdir");
    let mut store = DotallStore::init(temp.path()).expect("init");
    let nested = temp.path().join(".all/objects/sneaky.xlsx");
    fs::create_dir_all(nested.parent().expect("parent")).expect("dirs");
    fs::write(&nested, b"sneaky").expect("source");

    let error = store
        .register_source("./.all/objects/sneaky.xlsx", "xlsx")
        .expect_err("internal path");

    assert!(error.to_string().contains("invalid source path"));
}
