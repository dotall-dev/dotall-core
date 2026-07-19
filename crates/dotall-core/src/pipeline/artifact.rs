use serde::{Deserialize, Serialize};

use crate::registry::{ArtifactEnvelope, ReadResponse};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedArtifact {
    pub source_hash: String,
    pub producer_id: String,
    pub producer_version: String,
    pub artifact: ArtifactEnvelope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedView {
    pub source_hash: String,
    pub renderer_id: String,
    pub renderer_version: String,
    pub request_hash: String,
    pub response: ReadResponse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedDerived {
    pub source_hash: String,
    pub model_schema_id: String,
    pub model_schema_version: u32,
    pub processor_id: String,
    pub processor_version: String,
    pub payload: serde_json::Value,
}
