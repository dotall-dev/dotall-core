mod error;
mod fingerprint;
pub mod history;
mod manifest;
pub mod orchestrate;
mod status;
mod store;
mod workspace;

pub mod pipeline;
pub mod read;
pub mod registry;

pub use error::{DotallError, Result};
pub use fingerprint::{Freshness, SourceFingerprint, check_freshness, fingerprint};
pub use history::{
    ApplyLock, CancelAudit, CancelStatus, EditRequest, HistoryRecord, HistoryStatus,
    HistorySummary, StagedEdit,
};
pub use manifest::{MANIFEST_SCHEMA_VERSION, Manifest, ObjectMeta, OriginalRef, TrackedObject};
pub use orchestrate::{
    AppliedEdit, Engine, EngineStatus, FileReadResult, InspectResult, ModelLoadResult,
};
pub use pipeline::{CachedArtifact, CachedDerived, CachedView, DerivationRecipe};
pub use registry::{
    Actor, ActorKind, ArtifactEnvelope, ArtifactSchema, DependencyImpact, EncodedSnapshot,
    PatchedOutput, ReadResponse, SemanticChange, SemanticOperation, SnapshotPart, ValidatedEdit,
};
pub use status::{ObjectState, ObjectStatus};
pub use store::DotallStore;
pub use workspace::Workspace;

pub const ALL_DIR_NAME: &str = ".all";
