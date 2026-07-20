//! Edit staging, transaction journals, and forensic history records.
//!
//! Stage/apply orchestration lives in [`crate::orchestrate::Engine`]; format
//! handlers only validate operations and surgically patch source bytes.

mod lock;
mod record;
mod transaction;

pub use record::{HistoryRecord, HistoryStatus};
pub use transaction::{EditRequest, StagedEdit};

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::registry::{Actor, ActorKind, DependencyImpact, SemanticChange, SemanticOperation};

    use super::*;

    #[test]
    fn history_record_serde_round_trip() {
        let record = HistoryRecord {
            version: 1,
            tx_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid"),
            status: HistoryStatus::Applied,
            timestamp: "2026-07-20T10:15:30Z".into(),
            actor: Actor {
                kind: ActorKind::Cli,
                id: Some("dotall".into()),
            },
            ops: vec![SemanticOperation {
                kind: "set_cell_formula".into(),
                payload: serde_json::json!({
                    "sheet": "Revenue",
                    "address": "B12",
                    "formula": "=A1*1.1"
                }),
            }],
            semantic_diff: vec![SemanticChange {
                target: "Revenue!B12".into(),
                element_id: "c_rev_b12".into(),
                change: "formula".into(),
                before: Some("=A1".into()),
                after: Some("=A1*1.1".into()),
            }],
            dependency_impact: DependencyImpact {
                forward: vec!["Revenue!C12".into(), "Summary!B2".into()],
                notes: vec!["refs parsed; values not evaluated".into()],
            },
            before_source_hash: "abc123".into(),
            after_source_hash: "def456".into(),
            snapshot_ref: "snap_blake3".into(),
            revert_of: None,
        };

        let json = serde_json::to_string_pretty(&record).expect("serialize");
        let decoded: HistoryRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, decoded);
    }

    #[test]
    fn edit_request_serde_round_trip() {
        let request = EditRequest {
            transaction_id: Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").expect("uuid"),
            expected_source_hash: "expected_hash".into(),
            actor: Actor {
                kind: ActorKind::Mcp,
                id: None,
            },
            operations: vec![
                SemanticOperation {
                    kind: "set_cell_value".into(),
                    payload: serde_json::json!({
                        "sheet": "Revenue",
                        "address": "A1",
                        "value": 100
                    }),
                },
                SemanticOperation {
                    kind: "set_cell_formula".into(),
                    payload: serde_json::json!({
                        "element_id": "c_rev_b12",
                        "formula": "=A1*1.1"
                    }),
                },
            ],
        };

        let json = serde_json::to_string_pretty(&request).expect("serialize");
        let decoded: EditRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(request, decoded);
    }
}
