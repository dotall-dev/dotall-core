use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DetectionScore(pub u16);

pub struct DetectionProbe<'a> {
    pub path: &'a Path,
    pub prefix: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatDescriptor {
    pub id: String,
    pub version: String,
    pub capabilities: Vec<Capability>,
    pub edit_capabilities: Vec<EditCapability>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditCapability {
    pub operation: String,
    pub schema_version: u32,
    pub description: String,
    pub example: serde_json::Value,
    pub safety: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Inspect,
    ReadFull,
    ReadSelector { kind: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactEnvelope {
    pub format_id: String,
    pub schema_id: String,
    pub schema_version: u32,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactSchema {
    pub format_id: String,
    pub schema_id: String,
    pub schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inspection {
    pub format_id: String,
    pub summary: serde_json::Value,
    pub capabilities: Vec<Capability>,
    pub edit_capabilities: Vec<EditCapability>,
    pub suggested_reads: Vec<ReadSuggestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadSuggestion {
    pub description: String,
    pub selector: ReadSelector,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadSelector {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRequest {
    pub selector: Option<ReadSelector>,
    pub max_tokens: usize,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadResponse {
    pub content: String,
    pub estimated_tokens: usize,
    pub truncated: bool,
    pub continuation: Option<String>,
    pub next_actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Cli,
    Mcp,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Format-agnostic edit operation: `kind` selects the handler-specific payload shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticOperation {
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticChange {
    pub target: String,
    pub element_id: String,
    pub change: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyImpact {
    pub forward: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Handler-validated edit ready for staging or surgical apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidatedEdit {
    pub format_id: String,
    pub schema_id: String,
    pub schema_version: u32,
    pub operations: Vec<SemanticOperation>,
    pub semantic_diff: Vec<SemanticChange>,
    pub dependency_impact: DependencyImpact,
}

/// Result of a successful surgical patch; Engine atomically replaces the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchedOutput {
    pub bytes: Vec<u8>,
    pub after_source_hash: String,
}

/// Format-owned snapshot encoding persisted by the core object store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncodedSnapshot {
    pub package_hash: String,
    pub format_id: String,
    pub manifest: serde_json::Value,
    pub parts: Vec<SnapshotPart>,
}

/// One content-addressed blob referenced by an encoded snapshot manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPart {
    pub hash: String,
    pub bytes: Vec<u8>,
}
