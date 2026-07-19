use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::{DotallError, Result};

#[derive(Debug, Serialize)]
pub struct AccessRecord<'a> {
    pub operation: &'a str,
    pub path: &'a str,
    pub timestamp_unix_ms: u64,
    pub source_hash: &'a str,
    pub model_cache_hit: bool,
    pub view_cache_hit: bool,
    pub estimated_tokens: Option<usize>,
    pub truncated: Option<bool>,
}

pub fn now_unix_ms() -> Result<u64> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        DotallError::InvalidWorkspacePath("<system clock before Unix epoch>".into())
    })?;
    u64::try_from(duration.as_millis())
        .map_err(|_| DotallError::InvalidWorkspacePath("<system clock overflow>".into()))
}
