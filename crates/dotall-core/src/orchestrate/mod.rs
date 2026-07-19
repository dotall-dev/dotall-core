use std::fs::File;
use std::io::Read;
use std::path::{Component, Path};
use std::sync::Arc;

use serde::Serialize;

use crate::pipeline::CachedView;
use crate::read::{AccessRecord, apply_budget, now_unix_ms};
use crate::registry::{
    ArtifactEnvelope, FormatHandler, FormatRegistry, Inspection, ReadRequest, ReadResponse,
};
use crate::{DotallError, DotallStore, ObjectState, Result};

#[derive(Debug, Serialize)]
pub struct InspectResult {
    pub source_hash: String,
    pub model_cache_hit: bool,
    pub inspection: Inspection,
}

#[derive(Debug, Serialize)]
pub struct FileReadResult {
    pub source_hash: String,
    pub model_cache_hit: bool,
    pub view_cache_hit: bool,
    pub response: ReadResponse,
}

pub struct Engine {
    store: DotallStore,
    registry: FormatRegistry,
}

impl Engine {
    pub fn new(store: DotallStore, registry: FormatRegistry) -> Self {
        Self { store, registry }
    }

    pub fn inspect(&mut self, relative: &str) -> Result<InspectResult> {
        let (handler, model, model_cache_hit, source_hash) = self.model(relative)?;
        let inspection = handler.inspect(&model)?;
        self.store.append_access(
            relative,
            &AccessRecord {
                operation: "inspect",
                path: relative,
                timestamp_unix_ms: now_unix_ms()?,
                source_hash: &source_hash,
                model_cache_hit,
                view_cache_hit: false,
                estimated_tokens: None,
                truncated: None,
            },
        )?;

        Ok(InspectResult {
            source_hash,
            model_cache_hit,
            inspection,
        })
    }

    pub fn read(&mut self, relative: &str, request: &ReadRequest) -> Result<FileReadResult> {
        let (handler, model, model_cache_hit, source_hash) = self.model(relative)?;
        let descriptor = handler.descriptor();
        let request_hash = request_hash(&descriptor.id, &descriptor.version, request)?;
        let (response, view_cache_hit) = if let Some(view) =
            self.store.read_view(relative, &request_hash)?
        {
            if view.renderer_id == descriptor.id && view.renderer_version == descriptor.version {
                (view.response, true)
            } else {
                (self.render(&handler, &model, request)?, false)
            }
        } else {
            (self.render(&handler, &model, request)?, false)
        };

        if !view_cache_hit {
            self.store.write_view(
                relative,
                &CachedView {
                    source_hash: source_hash.clone(),
                    renderer_id: descriptor.id,
                    renderer_version: descriptor.version,
                    request_hash,
                    response: response.clone(),
                },
            )?;
        }

        self.store.append_access(
            relative,
            &AccessRecord {
                operation: "read",
                path: relative,
                timestamp_unix_ms: now_unix_ms()?,
                source_hash: &source_hash,
                model_cache_hit,
                view_cache_hit,
                estimated_tokens: Some(response.estimated_tokens),
                truncated: Some(response.truncated),
            },
        )?;

        Ok(FileReadResult {
            source_hash,
            model_cache_hit,
            view_cache_hit,
            response,
        })
    }

    fn render(
        &self,
        handler: &Arc<dyn FormatHandler>,
        model: &ArtifactEnvelope,
        request: &ReadRequest,
    ) -> Result<ReadResponse> {
        let mut render_request = request.clone();
        render_request.continuation = None;
        let mut response = handler.read(model, &render_request)?;
        let offset = parse_continuation(request, &handler.descriptor().id)?;
        let (content, truncated, continuation) =
            apply_budget(&response.content, request.max_tokens, offset);
        response.estimated_tokens = content.chars().count().div_ceil(4);
        response.content = content;
        response.truncated = truncated;
        response.continuation = continuation;
        Ok(response)
    }

    fn model(
        &mut self,
        relative: &str,
    ) -> Result<(Arc<dyn FormatHandler>, ArtifactEnvelope, bool, String)> {
        let relative_path = Path::new(relative);
        validate_relative(relative_path)?;
        let source = self.store.workspace().root().join(relative_path);
        let prefix = read_prefix(&source)?;
        let handler = self.registry.detect(relative_path, &prefix)?;

        let is_fresh = self.store.manifest().objects.contains_key(relative)
            && self
                .store
                .status()?
                .into_iter()
                .find(|object| object.path == relative)
                .is_some_and(|object| {
                    matches!(
                        object.state,
                        ObjectState::FreshFastPath | ObjectState::FreshAfterHash
                    )
                });
        if is_fresh && let Some(model) = self.store.read_model(relative)? {
            let source_hash = self.store.manifest().objects[relative]
                .fingerprint
                .blake3
                .clone();
            return Ok((handler, model, true, source_hash));
        }

        self.store
            .register_source(relative, handler.descriptor().id.clone())?;
        let model = handler.parse(&source)?;
        self.store.write_model(relative, &model)?;
        let source_hash = self.store.manifest().objects[relative]
            .fingerprint
            .blake3
            .clone();
        Ok((handler, model, false, source_hash))
    }
}

fn request_hash(
    renderer_id: &str,
    renderer_version: &str,
    request: &ReadRequest,
) -> Result<String> {
    let bytes =
        serde_json::to_vec(&(renderer_id, renderer_version, request)).map_err(|source| {
            DotallError::Serialization {
                context: "read request cache key".into(),
                source,
            }
        })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn parse_continuation(request: &ReadRequest, format_id: &str) -> Result<usize> {
    request.continuation.as_deref().map_or(Ok(0), |cursor| {
        cursor
            .parse()
            .map_err(|_| DotallError::UnsupportedCapability {
                format_id: format_id.into(),
                capability: "invalid continuation cursor".into(),
                available: vec!["reuse the cursor returned by the previous read".into()],
            })
    })
}

fn read_prefix(source: &Path) -> Result<Vec<u8>> {
    let mut file =
        File::open(source).map_err(|source_error| DotallError::io(source, source_error))?;
    let mut prefix = vec![0_u8; 16];
    let count = file
        .read(&mut prefix)
        .map_err(|source_error| DotallError::io(source, source_error))?;
    prefix.truncate(count);
    Ok(prefix)
}

fn validate_relative(relative: &Path) -> Result<()> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || relative
            .components()
            .any(|component| component.as_os_str() == ".all")
    {
        return Err(DotallError::InvalidSourcePath {
            path: relative.to_path_buf(),
            reason: "path must be relative, remain inside the workspace, and not target .all"
                .into(),
        });
    }
    Ok(())
}
