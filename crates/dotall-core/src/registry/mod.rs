//! Format registration and discovery contracts.
//!
//! Stage/apply orchestration is owned by [`crate::orchestrate::Engine`]; handlers
//! only validate semantic operations and surgically patch source bytes.

mod types;

use std::path::Path;
use std::sync::Arc;

use crate::{DotallError, Result};

pub use types::*;

pub trait FormatHandler: Send + Sync {
    fn descriptor(&self) -> FormatDescriptor;
    fn artifact_schema(&self) -> ArtifactSchema;
    fn detect(&self, probe: &DetectionProbe<'_>) -> DetectionScore;
    fn parse(&self, source: &Path) -> Result<ArtifactEnvelope>;
    fn inspect(&self, model: &ArtifactEnvelope) -> Result<Inspection>;
    fn read(&self, model: &ArtifactEnvelope, request: &ReadRequest) -> Result<ReadResponse>;

    fn validate_edit(
        &self,
        model: &ArtifactEnvelope,
        operations: &[SemanticOperation],
    ) -> Result<ValidatedEdit>;

    fn apply_edit(&self, source: &Path, edit: &ValidatedEdit) -> Result<PatchedOutput>;
}

#[derive(Default)]
pub struct FormatRegistry {
    handlers: Vec<Arc<dyn FormatHandler>>,
}

impl FormatRegistry {
    pub fn register(&mut self, handler: Arc<dyn FormatHandler>) {
        self.handlers.push(handler);
    }

    pub fn detect(&self, path: &Path, prefix: &[u8]) -> Result<Arc<dyn FormatHandler>> {
        let probe = DetectionProbe { path, prefix };

        self.handlers
            .iter()
            .map(|handler| (handler.detect(&probe), Arc::clone(handler)))
            .filter(|(score, _)| score.0 > 0)
            .max_by_key(|(score, _)| score.0)
            .map(|(_, handler)| handler)
            .ok_or_else(|| DotallError::UnsupportedFormat(path.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;

    struct Stub(&'static str, u16);

    impl FormatHandler for Stub {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor {
                id: self.0.into(),
                version: "1".into(),
                capabilities: vec![Capability::Inspect],
            }
        }

        fn artifact_schema(&self) -> ArtifactSchema {
            ArtifactSchema {
                format_id: self.0.into(),
                schema_id: format!("{}.document", self.0),
                schema_version: 1,
            }
        }

        fn detect(&self, _probe: &DetectionProbe<'_>) -> DetectionScore {
            DetectionScore(self.1)
        }

        fn parse(&self, _source: &Path) -> Result<ArtifactEnvelope> {
            unreachable!("selection test does not parse")
        }

        fn inspect(&self, _model: &ArtifactEnvelope) -> Result<Inspection> {
            unreachable!("selection test does not inspect")
        }

        fn read(&self, _model: &ArtifactEnvelope, _request: &ReadRequest) -> Result<ReadResponse> {
            unreachable!("selection test does not read")
        }

        fn validate_edit(
            &self,
            _model: &ArtifactEnvelope,
            _operations: &[SemanticOperation],
        ) -> Result<ValidatedEdit> {
            unreachable!("selection test does not validate edits")
        }

        fn apply_edit(&self, _source: &Path, _edit: &ValidatedEdit) -> Result<PatchedOutput> {
            unreachable!("selection test does not apply edits")
        }
    }

    #[test]
    fn highest_detection_score_wins_and_ignores_zero_scores() {
        let mut registry = FormatRegistry::default();
        registry.register(Arc::new(Stub("ignored", 0)));
        registry.register(Arc::new(Stub("weak", 10)));
        registry.register(Arc::new(Stub("strong", 80)));

        let selected = registry
            .detect(Path::new("book.bin"), b"bytes")
            .expect("selected");

        assert_eq!(selected.descriptor().id, "strong");
    }

    #[test]
    fn missing_detection_match_returns_unsupported_format() {
        let mut registry = FormatRegistry::default();
        registry.register(Arc::new(Stub("ignored", 0)));

        let error = match registry.detect(Path::new("book.bin"), b"bytes") {
            Err(error) => error,
            Ok(_) => panic!("no format should match"),
        };

        assert!(matches!(
            error,
            DotallError::UnsupportedFormat(path) if path == Path::new("book.bin")
        ));
    }
}
