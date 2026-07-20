use serde::{Deserialize, Serialize};

use crate::error::{DotallError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationRecipe {
    pub source_hash: String,
    pub processor_id: String,
    pub processor_version: String,
    pub config_hash: String,
    pub input_hashes: Vec<String>,
}

impl DerivationRecipe {
    pub fn key(&self) -> Result<String> {
        let bytes = serde_json::to_vec(self).map_err(|source| DotallError::Serialization {
            context: "derivation recipe".into(),
            source,
        })?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::DerivationRecipe;

    fn recipe() -> DerivationRecipe {
        DerivationRecipe {
            source_hash: "source".into(),
            processor_id: "xlsx.formula-dependencies".into(),
            processor_version: "1".into(),
            config_hash: "config".into(),
            input_hashes: vec!["model".into()],
        }
    }

    #[test]
    fn key_is_deterministic_and_changes_with_an_input() {
        let first = recipe();
        let mut changed = recipe();
        changed.processor_version = "2".into();

        assert_eq!(
            first.key().expect("first key"),
            recipe().key().expect("second key")
        );
        assert_ne!(
            first.key().expect("first key"),
            changed.key().expect("changed key")
        );
    }

    #[test]
    fn key_changes_when_any_field_changes() {
        let base = recipe();
        let base_key = base.key().expect("base key");

        let mut source = recipe();
        source.source_hash = "other-source".into();
        assert_ne!(base_key, source.key().expect("source key"));

        let mut processor_id = recipe();
        processor_id.processor_id = "other.processor".into();
        assert_ne!(base_key, processor_id.key().expect("processor_id key"));

        let mut processor_version = recipe();
        processor_version.processor_version = "99".into();
        assert_ne!(
            base_key,
            processor_version.key().expect("processor_version key")
        );

        let mut config = recipe();
        config.config_hash = "other-config".into();
        assert_ne!(base_key, config.key().expect("config key"));

        let mut inputs = recipe();
        inputs.input_hashes = vec!["model".into(), "extra".into()];
        assert_ne!(base_key, inputs.key().expect("input_hashes key"));
    }
}
