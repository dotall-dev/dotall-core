use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct InitParams {
    #[schemars(description = "Absolute or current-working-directory-relative workspace path")]
    pub workspace: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct StatusParams {
    #[schemars(description = "Absolute or current-working-directory-relative workspace path")]
    pub workspace: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct FileParams {
    #[schemars(description = "Path to a file inside an initialized Dotall workspace")]
    pub file: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct CapabilitiesParams {
    #[schemars(description = "Path to a file inside an initialized Dotall workspace")]
    pub file: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ReadParams {
    pub file: String,
    #[schemars(
        description = "Selector kind advertised by inspect, such as range, sheet, full, or dependencies"
    )]
    pub selector_kind: Option<String>,
    #[schemars(description = "Format-natural selector, such as Revenue!A1:D20")]
    pub selector: Option<String>,
    #[schemars(range(min = 1, max = 100_000))]
    pub max_tokens: Option<usize>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DepsParams {
    pub file: String,
    #[schemars(description = "A1-style cell address, such as Sheet1!B2")]
    pub cell: String,
    #[schemars(description = "When true, return dependents instead of precedents")]
    #[serde(default)]
    pub dependents: bool,
}

/// Stage-only edit request. Edits are never auto-applied; call `apply` or rely on flush-on-close.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct EditParams {
    pub file: String,
    #[schemars(description = "Source hash returned by inspect/status; prevents stale writes")]
    pub expected_source_hash: String,
    #[schemars(description = "Stable UUID reused when retrying the same edit")]
    pub transaction_id: Option<String>,
    pub actor_id: String,
    pub operations: Vec<OperationParam>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct OperationParam {
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct StagedParams {
    pub file: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ApplyParams {
    pub file: String,
    #[schemars(
        description = "Apply a single staged transaction; omit when applying all staged edits"
    )]
    pub transaction_id: Option<String>,
    #[schemars(description = "Apply every staged transaction for this file")]
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DiscardParams {
    pub file: String,
    #[schemars(description = "Staged transaction UUID to discard")]
    pub transaction_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct HistoryParams {
    pub file: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DiffParams {
    pub file: String,
    pub from_version: u64,
    pub to_version: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct RevertParams {
    pub file: String,
    pub version: u64,
    #[schemars(description = "Source hash returned by inspect/status; prevents stale writes")]
    pub expected_source_hash: String,
    #[schemars(description = "Stable UUID reused when retrying the same revert")]
    pub transaction_id: Option<String>,
    pub actor_id: String,
}
