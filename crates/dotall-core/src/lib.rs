mod error;
mod fingerprint;
mod manifest;
mod status;
mod store;
mod workspace;

pub mod read;
pub mod registry;

pub use error::{DotallError, Result};
pub use fingerprint::{Freshness, SourceFingerprint, check_freshness, fingerprint};
pub use manifest::{MANIFEST_SCHEMA_VERSION, Manifest, ObjectMeta, OriginalRef, TrackedObject};
pub use status::{ObjectState, ObjectStatus};
pub use store::DotallStore;
pub use workspace::Workspace;

pub const ALL_DIR_NAME: &str = ".all";
