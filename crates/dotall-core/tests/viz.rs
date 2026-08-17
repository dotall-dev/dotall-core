use std::fs;

use dotall_core::{
    Actor, ActorKind, DependencyImpact, DotallStore, HistoryRecord, HistoryStatus,
    SemanticOperation, VizNode, viz_snapshot,
};
use tempfile::tempdir;
use uuid::Uuid;

#[test]
fn viz_tree_lists_object_and_history() {
    let temp = tempdir().expect("temp");
    fs::write(temp.path().join("book.xlsx"), b"abcdef").expect("src");
    let mut store = DotallStore::init(temp.path()).expect("init");
    store.register_source("book.xlsx", "xlsx").expect("reg");
    store
        .append_history(
            "book.xlsx",
            &HistoryRecord {
                version: 0,
                tx_id: Uuid::nil(),
                status: HistoryStatus::Applied,
                timestamp: "2026-08-17T00:00:00Z".into(),
                actor: Actor {
                    kind: ActorKind::Cli,
                    id: Some("t".into()),
                },
                ops: vec![SemanticOperation {
                    kind: "set_cell_value".into(),
                    payload: serde_json::json!({"sheet":"Inputs","address":"B2"}),
                }],
                semantic_diff: vec![],
                dependency_impact: DependencyImpact {
                    forward: vec![],
                    notes: vec![],
                },
                before_source_hash: "a".into(),
                after_source_hash: "b".into(),
                snapshot_ref: "snap".into(),
                revert_of: None,
            },
        )
        .expect("history");

    let snap = viz_snapshot(&store).expect("viz");
    assert!(snap.metrics.all_bytes > 0);
    assert!(
        json_contains_name(&snap.tree, "book.xlsx") || json_contains_name(&snap.tree, "objects")
    );
    assert_eq!(snap.history.len(), 1);
    assert_eq!(snap.history[0].version, 1);
}

#[test]
fn viz_history_exposes_parent_and_revert_edges() {
    let temp = tempdir().expect("temp");
    fs::write(temp.path().join("book.xlsx"), b"abcdef").expect("src");
    let mut store = DotallStore::init(temp.path()).expect("init");
    store.register_source("book.xlsx", "xlsx").expect("reg");
    store
        .append_history("book.xlsx", &history_record(None))
        .expect("v1");
    store
        .append_history("book.xlsx", &history_record(Some(1)))
        .expect("v2");

    let snap = viz_snapshot(&store).expect("viz");
    assert_eq!(snap.history.len(), 2);
    assert_eq!(snap.history[0].version, 1);
    assert_eq!(snap.history[0].parent, None);
    assert_eq!(snap.history[0].revert_of, None);
    assert_eq!(snap.history[0].timestamp, "2026-08-17T00:00:00Z");
    assert_eq!(snap.history[1].version, 2);
    assert_eq!(snap.history[1].parent, Some(1));
    assert_eq!(snap.history[1].revert_of, Some(1));
}

#[test]
fn viz_two_objects_expose_cache_hit_metric_fields() {
    let temp = tempdir().expect("temp");
    fs::write(temp.path().join("a.xlsx"), b"aaaaaa").expect("a");
    fs::write(temp.path().join("b.xlsx"), b"bbbbbb").expect("b");
    let mut store = DotallStore::init(temp.path()).expect("init");
    store.register_source("a.xlsx", "xlsx").expect("reg a");
    store.register_source("b.xlsx", "xlsx").expect("reg b");
    store
        .append_history(
            "a.xlsx",
            &HistoryRecord {
                version: 0,
                tx_id: Uuid::nil(),
                status: HistoryStatus::Applied,
                timestamp: "2026-08-17T00:00:00Z".into(),
                actor: Actor {
                    kind: ActorKind::Cli,
                    id: Some("t".into()),
                },
                ops: vec![SemanticOperation {
                    kind: "set_cell_value".into(),
                    payload: serde_json::json!({"sheet":"Inputs","address":"B2"}),
                }],
                semantic_diff: vec![],
                dependency_impact: DependencyImpact {
                    forward: vec![],
                    notes: vec![],
                },
                before_source_hash: "a".into(),
                after_source_hash: "b".into(),
                snapshot_ref: "snap".into(),
                revert_of: None,
            },
        )
        .expect("history");

    // Corrupt access-log line must not fail the whole viz.
    let access = temp
        .path()
        .join(".all/objects/a.xlsx/state/access/log.jsonl");
    fs::create_dir_all(access.parent().unwrap()).expect("access dir");
    fs::write(
        &access,
        b"{not-json\n{\"model_cache_hit\":true,\"view_cache_hit\":false,\"estimated_tokens\":3}\n",
    )
    .expect("access log");

    let snap = viz_snapshot(&store).expect("viz");
    assert_eq!(snap.history.len(), 1);
    assert!(json_contains_name(&snap.tree, "a.xlsx") || json_contains_name(&snap.tree, "objects"));
    // Fields must exist on VizMetrics (serde already serializes them; assert values).
    let _ = snap.metrics.model_cache_hits;
    let _ = snap.metrics.view_cache_hits;
    assert!(snap.metrics.model_cache_hits >= 1);
    assert_eq!(snap.metrics.view_cache_hits, 0);
}

fn json_contains_name(node: &VizNode, name: &str) -> bool {
    node.name == name || node.children.iter().any(|c| json_contains_name(c, name))
}

fn history_record(revert_of: Option<u64>) -> HistoryRecord {
    HistoryRecord {
        version: 0,
        tx_id: Uuid::nil(),
        status: HistoryStatus::Applied,
        timestamp: "2026-08-17T00:00:00Z".into(),
        actor: Actor {
            kind: ActorKind::Cli,
            id: Some("t".into()),
        },
        ops: vec![SemanticOperation {
            kind: "set_cell_value".into(),
            payload: serde_json::json!({"sheet":"Inputs","address":"B2"}),
        }],
        semantic_diff: vec![],
        dependency_impact: DependencyImpact {
            forward: vec![],
            notes: vec![],
        },
        before_source_hash: "a".into(),
        after_source_hash: "b".into(),
        snapshot_ref: "snap".into(),
        revert_of,
    }
}
