use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::registry::{Actor, SemanticOperation, ValidatedEdit};

/// Audit record written when a staged edit is discarded without apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelAudit {
    #[serde(rename = "tx_id")]
    pub tx_id: Uuid,
    pub status: CancelStatus,
    pub timestamp: String,
    pub actor: Actor,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelStatus {
    Cancelled,
}

/// Agent request to stage one or more semantic operations against a tracked object.
///
/// Staging is the default path; apply is a separate Engine call. There is no
/// auto-apply flag on this envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditRequest {
    pub transaction_id: Uuid,
    pub expected_source_hash: String,
    pub actor: Actor,
    pub operations: Vec<SemanticOperation>,
}

/// Durable staged edit awaiting explicit apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StagedEdit {
    #[serde(rename = "tx_id")]
    pub tx_id: Uuid,
    pub preview: ValidatedEdit,
    pub staged_at: String,
    pub expected_source_hash: String,
    pub actor: Actor,
}

impl StagedEdit {
    /// Whether two staged envelopes carry the same durable payload for idempotent retries.
    pub fn same_payload(&self, other: &Self) -> bool {
        self.tx_id == other.tx_id
            && self.preview == other.preview
            && self.expected_source_hash == other.expected_source_hash
            && self.actor == other.actor
    }
}
