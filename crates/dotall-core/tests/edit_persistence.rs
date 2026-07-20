use std::fs;

use dotall_core::{
    Actor, ActorKind, DependencyImpact, DotallError, DotallStore, SemanticChange,
    SemanticOperation, StagedEdit, ValidatedEdit,
};
use tempfile::tempdir;
use uuid::Uuid;

fn sample_staged(tx_id: Uuid, expected_hash: &str, formula: &str) -> StagedEdit {
    StagedEdit {
        tx_id,
        preview: ValidatedEdit {
            format_id: "xlsx".into(),
            schema_id: "xlsx.cell-edits".into(),
            schema_version: 1,
            operations: vec![SemanticOperation {
                kind: "set_cell_formula".into(),
                payload: serde_json::json!({
                    "sheet": "Revenue",
                    "address": "B12",
                    "formula": formula
                }),
            }],
            semantic_diff: vec![SemanticChange {
                target: "Revenue!B12".into(),
                element_id: "c_rev_b12".into(),
                change: "formula".into(),
                before: Some("=A1".into()),
                after: Some(formula.into()),
            }],
            dependency_impact: DependencyImpact {
                forward: vec!["Revenue!C12".into()],
                notes: vec![],
            },
        },
        staged_at: "2026-07-20T10:00:00Z".into(),
        expected_source_hash: expected_hash.into(),
        actor: Actor {
            kind: ActorKind::Cli,
            id: Some("dotall".into()),
        },
    }
}

fn init_store_with_source() -> (tempfile::TempDir, DotallStore, Uuid) {
    let temp = tempdir().expect("tempdir");
    let source = temp.path().join("book.xlsx");
    fs::write(&source, b"workbook-bytes-v1").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("init");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register");
    let tx_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid");
    (temp, store, tx_id)
}

#[test]
fn stage_edit_writes_staging_file() {
    let (temp, store, tx_id) = init_store_with_source();
    let staged = sample_staged(tx_id, "hash_v1", "=A1*1.1");

    store.stage_edit("book.xlsx", &staged).expect("stage edit");

    let staging_path = temp.path().join(
        ".all/objects/book.xlsx/state/edits/staging/550e8400-e29b-41d4-a716-446655440000.json",
    );
    assert!(staging_path.is_file());
    let loaded = store
        .read_staged("book.xlsx", tx_id)
        .expect("read staged")
        .expect("staged exists");
    assert_eq!(loaded, staged);
}

#[test]
fn duplicate_stage_with_same_payload_is_idempotent() {
    let (_temp, store, tx_id) = init_store_with_source();
    let staged = sample_staged(tx_id, "hash_v1", "=A1*1.1");
    let retry = StagedEdit {
        staged_at: "2026-07-20T10:05:00Z".into(),
        ..staged.clone()
    };

    store.stage_edit("book.xlsx", &staged).expect("first stage");
    store
        .stage_edit("book.xlsx", &retry)
        .expect("duplicate stage");

    let loaded = store
        .read_staged("book.xlsx", tx_id)
        .expect("read staged")
        .expect("staged exists");
    assert_eq!(loaded.preview, staged.preview);
    assert_eq!(loaded.expected_source_hash, staged.expected_source_hash);
    assert_eq!(loaded.actor, staged.actor);
}

#[test]
fn duplicate_stage_with_conflicting_payload_errors() {
    let (_temp, store, tx_id) = init_store_with_source();
    let first = sample_staged(tx_id, "hash_v1", "=A1*1.1");
    let conflicting = sample_staged(tx_id, "hash_v1", "=A1*2.0");

    store.stage_edit("book.xlsx", &first).expect("first stage");
    let error = store
        .stage_edit("book.xlsx", &conflicting)
        .expect_err("conflicting stage");

    assert!(matches!(error, DotallError::StagedConflict { .. }));
}

#[test]
fn list_and_discard_staged_edits() {
    let (_temp, store, tx_id) = init_store_with_source();
    let other_id = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").expect("uuid");
    let first = sample_staged(tx_id, "hash_v1", "=A1*1.1");
    let second = sample_staged(other_id, "hash_v1", "=B1+1");

    store.stage_edit("book.xlsx", &first).expect("stage first");
    store
        .stage_edit("book.xlsx", &second)
        .expect("stage second");

    let mut listed = store.list_staged("book.xlsx").expect("list staged");
    listed.sort_by_key(|edit| edit.tx_id);
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].tx_id, tx_id);
    assert_eq!(listed[1].tx_id, other_id);

    store.discard_staged("book.xlsx", tx_id).expect("discard");
    assert!(
        store
            .read_staged("book.xlsx", tx_id)
            .expect("read staged")
            .is_none()
    );
    assert!(
        store
            .read_staged("book.xlsx", other_id)
            .expect("read other")
            .is_some()
    );
}

#[test]
fn apply_lock_blocks_concurrent_acquire() {
    use dotall_core::ApplyLock;

    let (temp, store, _tx_id) = init_store_with_source();
    let lock_path = temp
        .path()
        .join(".all/objects/book.xlsx/state/edits/.apply.lock");

    let first = store.acquire_apply_lock("book.xlsx").expect("first lock");
    assert!(lock_path.is_file());

    let second = store.acquire_apply_lock("book.xlsx");
    assert!(matches!(second, Err(DotallError::LockBusy { .. })));

    drop(first);
    ApplyLock::acquire(&lock_path).expect("lock after release");
}

#[test]
fn write_snapshot_is_content_addressed_and_idempotent() {
    let (temp, store, _tx_id) = init_store_with_source();
    let bytes = b"workbook-bytes-v1";

    let hash = store
        .write_snapshot("book.xlsx", bytes)
        .expect("first snapshot");
    let again = store
        .write_snapshot("book.xlsx", bytes)
        .expect("duplicate snapshot");
    assert_eq!(hash, again);

    let snapshot_path = temp.path().join(format!(
        ".all/objects/book.xlsx/state/edits/history/snapshots/{hash}.bin"
    ));
    assert!(snapshot_path.is_file());
    assert_eq!(fs::read(&snapshot_path).expect("read snapshot"), bytes);

    let entries: Vec<_> = fs::read_dir(
        temp.path()
            .join(".all/objects/book.xlsx/state/edits/history/snapshots"),
    )
    .expect("read snapshots dir")
    .collect();
    assert_eq!(entries.len(), 1);
}
