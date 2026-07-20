use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::registry::{Actor, SemanticOperation, ValidatedEdit};

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
