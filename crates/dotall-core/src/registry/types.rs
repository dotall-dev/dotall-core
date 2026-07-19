use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DetectionScore(pub u16);

pub struct DetectionProbe<'a> {
    pub path: &'a Path,
    pub prefix: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatDescriptor {
    pub id: String,
    pub version: String,
    pub capabilities: Vec<Capability>,
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
