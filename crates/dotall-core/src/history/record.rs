use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::registry::{Actor, DependencyImpact, SemanticChange, SemanticOperation};

/// Compact agent-facing history entry returned by [`crate::DotallStore::list_history`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySummary {
    pub version: u64,
    pub timestamp: String,
    pub actor: Actor,
    pub summary: String,
    pub op_count: usize,
}

/// Forensic committed edit version stored under `state/edits/history/vNNN.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub version: u64,
    #[serde(rename = "tx_id")]
    pub tx_id: Uuid,
    pub status: HistoryStatus,
    pub timestamp: String,
    pub actor: Actor,
    pub ops: Vec<SemanticOperation>,
    pub semantic_diff: Vec<SemanticChange>,
    pub dependency_impact: DependencyImpact,
    pub before_source_hash: String,
    pub after_source_hash: String,
    pub snapshot_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert_of: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Applied,
    Failed,
}

impl HistoryRecord {
    pub fn summary(&self) -> HistorySummary {
        HistorySummary {
            version: self.version,
            timestamp: self.timestamp.clone(),
            actor: self.actor.clone(),
            summary: one_line_summary(self),
            op_count: self.ops.len(),
        }
    }
}

pub(crate) fn history_version_file_name(version: u64) -> String {
    format!("v{version:03}.json")
}

fn one_line_summary(record: &HistoryRecord) -> String {
    let Some(op) = record.ops.first() else {
        return "no operations".into();
    };

    let target = record
        .semantic_diff
        .first()
        .map(|change| change.target.clone())
        .or_else(|| operation_target(&op.payload))
        .unwrap_or_else(|| "unknown target".into());

    format!("{} {}", op.kind, target)
}

fn operation_target(payload: &serde_json::Value) -> Option<String> {
    if let Some(element_id) = payload.get("element_id").and_then(|value| value.as_str()) {
        return Some(element_id.into());
    }

    let sheet = payload.get("sheet").and_then(|value| value.as_str())?;
    let address = payload.get("address").and_then(|value| value.as_str())?;
    Some(format!("{sheet}!{address}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{ActorKind, DependencyImpact, SemanticChange, SemanticOperation};

    #[test]
    fn summary_uses_first_op_and_semantic_diff_target() {
        let record = HistoryRecord {
            version: 1,
            tx_id: Uuid::nil(),
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
                forward: vec![],
                notes: vec![],
            },
            before_source_hash: "before".into(),
            after_source_hash: "after".into(),
            snapshot_ref: "snap".into(),
            revert_of: None,
        };

        let summary = record.summary();
        assert_eq!(summary.version, 1);
        assert_eq!(summary.op_count, 1);
        assert_eq!(summary.summary, "set_cell_formula Revenue!B12");
    }
}
