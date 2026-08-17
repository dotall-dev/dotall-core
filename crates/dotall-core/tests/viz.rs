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

fn json_contains_name(node: &VizNode, name: &str) -> bool {
    node.name == name || node.children.iter().any(|c| json_contains_name(c, name))
}
