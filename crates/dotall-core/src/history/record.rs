use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::registry::{Actor, DependencyImpact, SemanticChange, SemanticOperation};

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
