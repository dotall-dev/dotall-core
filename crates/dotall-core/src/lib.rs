mod error;
mod fingerprint;
mod manifest;
mod status;
mod store;
mod workspace;

pub use error::{DotallError, Result};
pub use fingerprint::{
    check_freshness, fingerprint, Freshness, SourceFingerprint,
};
pub use manifest::{
    Manifest, ObjectMeta, OriginalRef, TrackedObject, MANIFEST_SCHEMA_VERSION,
};
pub use status::{ObjectState, ObjectStatus};
pub use store::DotallStore;
pub use workspace::Workspace;

pub const ALL_DIR_NAME: &str = ".all";
