use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{DotallError, Result};
use crate::fingerprint::SourceFingerprint;

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub objects: BTreeMap<String, TrackedObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedObject {
    pub format_id: String,
    pub fingerprint: SourceFingerprint,
    pub version_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMeta {
    pub schema_version: u32,
    pub format_id: String,
    pub fingerprint: SourceFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginalRef {
    pub relative_path: String,
    pub source_hash: String,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            objects: BTreeMap::new(),
        }
    }
}

impl Manifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(DotallError::UnsupportedManifestSchema {
                found: self.schema_version,
                supported: MANIFEST_SCHEMA_VERSION,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Manifest, MANIFEST_SCHEMA_VERSION};

    #[test]
    fn default_manifest_uses_current_schema_and_no_objects() {
        let manifest = Manifest::default();

        assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION);
        assert!(manifest.objects.is_empty());
        manifest.validate().expect("valid manifest");
    }

    #[test]
    fn rejects_unknown_schema() {
        let manifest = Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION + 1,
            ..Manifest::default()
        };

        let error = manifest.validate().expect_err("unsupported schema");

        assert!(error.to_string().contains("unsupported manifest schema"));
    }
}
