use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

pub type Result<T> = std::result::Result<T, DotallError>;

#[derive(Debug, Error)]
pub enum DotallError {
    #[error("I/O operation failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("Dotall is not initialized at or above {0}")]
    WorkspaceNotInitialized(PathBuf),

    #[error("invalid workspace path: {0}")]
    InvalidWorkspacePath(PathBuf),

    #[error("invalid source path {path}: {reason}")]
    InvalidSourcePath { path: PathBuf, reason: String },

    #[error("manifest at {path} is invalid: {source}")]
    InvalidManifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("unsupported manifest schema {found}; this build supports {supported}")]
    UnsupportedManifestSchema { found: u32, supported: u32 },

    #[error("source changed while it was being hashed: {0}")]
    SourceChangedDuringRead(PathBuf),

    #[error("source hash mismatch for {path}: expected {expected}, found {actual}")]
    SourceHashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    #[error("unsupported file format: {0}")]
    UnsupportedFormat(PathBuf),

    #[error("format {format_id} rejected artifact schema {schema_id} v{schema_version}")]
    ArtifactSchemaMismatch {
        format_id: String,
        schema_id: String,
        schema_version: u32,
    },

    #[error("unsupported {format_id} capability {capability}; available: {available:?}")]
    UnsupportedCapability {
        format_id: String,
        capability: String,
        available: Vec<String>,
    },

    #[error("{format_id} processing failed for {path}: {message}")]
    Format {
        format_id: String,
        path: PathBuf,
        message: String,
    },

    #[error("failed to serialize {context}: {source}")]
    Serialization {
        context: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("staged edit {tx_id} for {path} conflicts with an existing staged payload")]
    StagedConflict { path: PathBuf, tx_id: String },

    #[error("apply lock is held for {path}")]
    LockBusy { path: PathBuf },

    #[error("snapshot {hash} is missing for {path}")]
    SnapshotMissing { path: PathBuf, hash: String },

    #[error("history version {version} already exists for {path}")]
    HistoryVersionExists { path: PathBuf, version: u64 },

    #[error("history version {version} is missing for {path}")]
    HistoryVersionMissing { path: PathBuf, version: u64 },
}

impl DotallError {
    pub(crate) fn io(path: impl AsRef<Path>, source: io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}
