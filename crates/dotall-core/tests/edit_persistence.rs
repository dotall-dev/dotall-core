use std::fs;

use dotall_core::{
    Actor, ActorKind, CancelAudit, CancelStatus, DependencyImpact, DotallError, DotallStore,
    HistoryRecord, HistoryStatus, SemanticChange, SemanticOperation, StagedEdit, ValidatedEdit,
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
fn discard_staged_writes_cancel_audit() {
    let (temp, store, tx_id) = init_store_with_source();
    let staged = sample_staged(tx_id, "hash_v1", "=A1*1.1");

    store.stage_edit("book.xlsx", &staged).expect("stage");
    store.discard_staged("book.xlsx", tx_id).expect("discard");

    let cancel_path = temp.path().join(format!(
        ".all/objects/book.xlsx/state/transactions/{tx_id}.cancel.json"
    ));
    assert!(cancel_path.is_file());
    let audit: CancelAudit =
        serde_json::from_slice(&fs::read(&cancel_path).expect("read cancel audit"))
            .expect("parse cancel audit");
    assert_eq!(audit.tx_id, tx_id);
    assert_eq!(audit.status, CancelStatus::Cancelled);
    assert_eq!(audit.actor, staged.actor);
    assert_eq!(audit.reason, "discard");
    assert!(!audit.timestamp.is_empty());

    let history = store.list_history("book.xlsx").expect("list history");
    assert!(history.is_empty());
}

#[test]
fn discard_missing_staged_edit_errors() {
    let (_temp, store, tx_id) = init_store_with_source();
    let err = store
        .discard_staged("book.xlsx", tx_id)
        .expect_err("discard missing");

    assert!(
        matches!(
            err,
            DotallError::StagedMissing {
                ref path,
                ref tx_id
            } if path == &std::path::PathBuf::from("book.xlsx")
                && tx_id == "550e8400-e29b-41d4-a716-446655440000"
        ),
        "expected StagedMissing, got {err:?}"
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

fn sample_history_record(
    version: u64,
    tx_id: Uuid,
    formula: &str,
    timestamp: &str,
    before_hash: &str,
    after_hash: &str,
) -> HistoryRecord {
    HistoryRecord {
        version,
        tx_id,
        status: HistoryStatus::Applied,
        timestamp: timestamp.into(),
        actor: Actor {
            kind: ActorKind::Cli,
            id: Some("dotall".into()),
        },
        ops: vec![SemanticOperation {
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
        before_source_hash: before_hash.into(),
        after_source_hash: after_hash.into(),
        snapshot_ref: "snap_hash".into(),
        revert_of: None,
    }
}

#[test]
fn append_history_writes_sequential_versions() {
    let (temp, mut store, tx_id) = init_store_with_source();
    let v1 = sample_history_record(
        0,
        tx_id,
        "=A1*1.1",
        "2026-07-20T10:15:30Z",
        "abc123",
        "def456",
    );

    let version = store.append_history("book.xlsx", &v1).expect("append v1");
    assert_eq!(version, 1);

    let other_id = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").expect("uuid");
    let v2 = sample_history_record(
        0,
        other_id,
        "=A1*2.0",
        "2026-07-20T11:00:00Z",
        "def456",
        "ghi789",
    );
    let version2 = store.append_history("book.xlsx", &v2).expect("append v2");
    assert_eq!(version2, 2);

    assert!(
        temp.path()
            .join(".all/objects/book.xlsx/state/edits/history/v001.json")
            .is_file()
    );
    assert!(
        temp.path()
            .join(".all/objects/book.xlsx/state/edits/history/v002.json")
            .is_file()
    );
    assert_eq!(
        store
            .manifest()
            .objects
            .get("book.xlsx")
            .expect("tracked")
            .version_count,
        2
    );
}

#[test]
fn list_history_returns_compact_summaries() {
    let (_temp, mut store, tx_id) = init_store_with_source();
    let v1 = sample_history_record(
        0,
        tx_id,
        "=A1*1.1",
        "2026-07-20T10:15:30Z",
        "abc123",
        "def456",
    );
    let other_id = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").expect("uuid");
    let v2 = sample_history_record(
        0,
        other_id,
        "=A1*2.0",
        "2026-07-20T11:00:00Z",
        "def456",
        "ghi789",
    );

    store.append_history("book.xlsx", &v1).expect("append v1");
    store.append_history("book.xlsx", &v2).expect("append v2");

    let summaries = store.list_history("book.xlsx").expect("list history");
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].version, 1);
    assert_eq!(summaries[0].timestamp, "2026-07-20T10:15:30Z");
    assert_eq!(summaries[0].actor.kind, ActorKind::Cli);
    assert_eq!(summaries[0].op_count, 1);
    assert_eq!(summaries[0].summary, "set_cell_formula Revenue!B12");
    assert_eq!(summaries[1].version, 2);
    assert_eq!(summaries[1].summary, "set_cell_formula Revenue!B12");
}

#[test]
fn get_history_returns_forensic_record() {
    let (_temp, mut store, tx_id) = init_store_with_source();
    let v1 = sample_history_record(
        0,
        tx_id,
        "=A1*1.1",
        "2026-07-20T10:15:30Z",
        "abc123",
        "def456",
    );
    store.append_history("book.xlsx", &v1).expect("append v1");

    let loaded = store.get_history("book.xlsx", 1).expect("get history");
    assert_eq!(loaded.version, 1);
    assert_eq!(loaded.tx_id, tx_id);
    assert_eq!(loaded.status, HistoryStatus::Applied);
    assert_eq!(loaded.timestamp, "2026-07-20T10:15:30Z");
    assert_eq!(loaded.ops, v1.ops);
    assert_eq!(loaded.semantic_diff, v1.semantic_diff);
    assert_eq!(loaded.dependency_impact, v1.dependency_impact);
    assert_eq!(loaded.before_source_hash, "abc123");
    assert_eq!(loaded.after_source_hash, "def456");
    assert_eq!(loaded.snapshot_ref, "snap_hash");
}

#[test]
fn append_history_rejects_existing_version_file() {
    let (_temp, mut store, tx_id) = init_store_with_source();
    let v1 = sample_history_record(
        0,
        tx_id,
        "=A1*1.1",
        "2026-07-20T10:15:30Z",
        "abc123",
        "def456",
    );

    let history_path = store
        .workspace()
        .root()
        .join(".all/objects/book.xlsx/state/edits/history/v001.json");
    fs::create_dir_all(history_path.parent().expect("history parent")).expect("history dir");
    fs::write(&history_path, b"{}\n").expect("pre-create version file");

    let err = store
        .append_history("book.xlsx", &v1)
        .expect_err("append should fail");
    assert!(
        matches!(
            err,
            DotallError::HistoryVersionExists {
                ref path,
                version: 1
            } if path == &std::path::PathBuf::from("book.xlsx")
        ),
        "expected HistoryVersionExists, got {err:?}"
    );
}

#[test]
fn discard_staged_does_not_append_history() {
    let (_temp, store, tx_id) = init_store_with_source();
    let staged = sample_staged(tx_id, "hash_v1", "=A1*1.1");

    store.stage_edit("book.xlsx", &staged).expect("stage");
    store
        .discard_staged("book.xlsx", tx_id)
        .expect("discard staged");

    let history = store.list_history("book.xlsx").expect("list history");
    assert!(history.is_empty());
}
