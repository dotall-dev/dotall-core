use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::history::{EditRequest, HistoryRecord, HistoryStatus, HistorySummary, StagedEdit};
use crate::pipeline::CachedView;
use crate::read::{AccessRecord, apply_budget, now_unix_ms};
use crate::registry::{
    Actor, ActorKind, ArtifactEnvelope, DependencyImpact, FormatHandler, FormatRegistry,
    Inspection, ReadRequest, ReadResponse, SemanticOperation, ValidatedEdit,
};
use crate::store::resolve_source;
use crate::{DotallError, DotallStore, ObjectState, Result, fingerprint};

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

#[derive(Debug, Clone, Serialize)]
pub struct ModelLoadResult {
    pub envelope: ArtifactEnvelope,
    pub source_hash: String,
    pub model_cache_hit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppliedEdit {
    pub tx_id: Uuid,
    pub version: u64,
    pub before_source_hash: String,
    pub after_source_hash: String,
    pub revert_of: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyJournal {
    tx_id: Uuid,
    before_source_hash: String,
    after_source_hash: String,
    snapshot_ref: String,
    committed: bool,
}

pub struct Engine {
    store: DotallStore,
    registry: FormatRegistry,
}

struct LoadedModel {
    key: String,
    handler: Arc<dyn FormatHandler>,
    model: ArtifactEnvelope,
    model_cache_hit: bool,
    source_hash: String,
}

impl Engine {
    pub fn new(store: DotallStore, registry: FormatRegistry) -> Self {
        Self { store, registry }
    }

    pub fn inspect(&mut self, relative: &str) -> Result<InspectResult> {
        let LoadedModel {
            key,
            handler,
            model,
            model_cache_hit,
            source_hash,
        } = self.model(relative)?;
        let inspection = handler.inspect(&model)?;
        self.store.append_access(
            &key,
            &AccessRecord {
                operation: "inspect",
                path: &key,
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

    pub fn load_model(&mut self, relative: &str) -> Result<ModelLoadResult> {
        let LoadedModel {
            model,
            source_hash,
            model_cache_hit,
            ..
        } = self.model(relative)?;

        Ok(ModelLoadResult {
            envelope: model,
            source_hash,
            model_cache_hit,
        })
    }

    pub fn read(&mut self, relative: &str, request: &ReadRequest) -> Result<FileReadResult> {
        let LoadedModel {
            key,
            handler,
            model,
            model_cache_hit,
            source_hash,
        } = self.model(relative)?;
        let descriptor = handler.descriptor();
        let request_hash = request_hash(&descriptor.id, &descriptor.version, request)?;
        let (response, view_cache_hit) = if let Some(view) =
            self.store.read_view(&key, &request_hash)?
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
                &key,
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
            &key,
            &AccessRecord {
                operation: "read",
                path: &key,
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

    /// Validates and durably stages an edit without changing the source bytes.
    pub fn edit(&mut self, relative: &str, request: &EditRequest) -> Result<StagedEdit> {
        let loaded = self.model(relative)?;
        if loaded.source_hash != request.expected_source_hash {
            return Err(DotallError::SourceHashMismatch {
                path: Path::new(relative).to_path_buf(),
                expected: request.expected_source_hash.clone(),
                actual: loaded.source_hash,
            });
        }

        let preview = loaded
            .handler
            .validate_edit(&loaded.model, &request.operations)?;
        let staged = StagedEdit {
            tx_id: request.transaction_id,
            preview,
            staged_at: now_unix_ms()?.to_string(),
            expected_source_hash: request.expected_source_hash.clone(),
            actor: request.actor.clone(),
        };
        if let Some(existing) = self.store.read_staged(relative, request.transaction_id)?
            && existing.same_payload(&staged)
        {
            return Ok(existing);
        }
        self.store.stage_edit(relative, &staged)?;
        Ok(staged)
    }

    pub fn apply(&mut self, relative: &str, tx_id: Uuid) -> Result<AppliedEdit> {
        self.apply_inner(relative, tx_id, false, false)
    }

    #[doc(hidden)]
    pub fn simulate_interruption_after_commit(
        &mut self,
        relative: &str,
        tx_id: Uuid,
    ) -> Result<AppliedEdit> {
        self.apply_inner(relative, tx_id, false, true)
    }

    #[doc(hidden)]
    pub fn simulate_interruption_after_replace(
        &mut self,
        relative: &str,
        tx_id: Uuid,
    ) -> Result<AppliedEdit> {
        self.apply_inner(relative, tx_id, true, false)
    }

    pub fn discard(&self, relative: &str, tx_id: Uuid) -> Result<()> {
        self.store.discard_staged(relative, tx_id)
    }

    pub fn staged(&self, relative: &str) -> Result<Vec<StagedEdit>> {
        self.store.list_staged(relative)
    }

    pub fn history(&self, relative: &str) -> Result<Vec<HistorySummary>> {
        self.store.list_history(relative)
    }

    pub fn diff(&self, relative: &str, version: u64) -> Result<HistoryRecord> {
        self.store.get_history(relative, version)
    }

    /// Stages restoration of the pre-apply snapshot from `version`.
    pub fn revert(&mut self, relative: &str, version: u64, tx_id: Uuid) -> Result<StagedEdit> {
        let record = self.store.get_history(relative, version)?;
        self.store.read_snapshot(relative, &record.snapshot_ref)?;
        let loaded = self.model(relative)?;
        let staged = StagedEdit {
            tx_id,
            preview: ValidatedEdit {
                format_id: loaded.model.format_id,
                schema_id: "dotall.restore-snapshot".into(),
                schema_version: 1,
                operations: vec![SemanticOperation {
                    kind: "restore_snapshot".into(),
                    payload: serde_json::json!({
                        "snapshot_ref": record.snapshot_ref,
                        "revert_of": version,
                    }),
                }],
                semantic_diff: record.semantic_diff,
                dependency_impact: DependencyImpact {
                    forward: Vec::new(),
                    notes: vec!["restores the pre-apply snapshot".into()],
                },
            },
            staged_at: now_unix_ms()?.to_string(),
            expected_source_hash: loaded.source_hash,
            actor: Actor {
                kind: ActorKind::System,
                id: Some("revert".into()),
            },
        };
        if let Some(existing) = self.store.read_staged(relative, tx_id)?
            && existing.same_payload(&staged)
        {
            return Ok(existing);
        }
        self.store.stage_edit(relative, &staged)?;
        Ok(staged)
    }

    /// Finalizes journals whose source replacement is already durable.
    pub fn recover(&mut self, relative: &str) -> Result<usize> {
        let journals: Vec<ApplyJournal> = self.store.read_journals(relative)?;
        let mut recovered = 0;
        for journal in journals {
            let _lock = self.store.acquire_apply_lock(relative)?;
            let (_, source) = resolve_source(self.store.workspace(), Path::new(relative))?;
            let actual_hash = fingerprint(&source)?.blake3;
            if actual_hash == journal.after_source_hash {
                let Some(staged) = self.store.read_staged(relative, journal.tx_id)? else {
                    self.store.discard_journal(relative, journal.tx_id)?;
                    continue;
                };
                let loaded = self.model(relative)?;
                self.finalize_apply(relative, &loaded.handler, &staged, &journal)?;
                recovered += 1;
            } else if !journal.committed && actual_hash == journal.before_source_hash {
                self.store.discard_journal(relative, journal.tx_id)?;
            }
        }
        Ok(recovered)
    }

    fn apply_inner(
        &mut self,
        relative: &str,
        tx_id: Uuid,
        interrupt_after_replace: bool,
        interrupt_after_commit: bool,
    ) -> Result<AppliedEdit> {
        if let Some(record) = self.history_for_transaction(relative, tx_id)? {
            return Ok(applied_from_record(&record));
        }
        let staged = self.store.read_staged(relative, tx_id)?.ok_or_else(|| {
            DotallError::InvalidSourcePath {
                path: Path::new(relative).to_path_buf(),
                reason: format!("staged edit {tx_id} is missing"),
            }
        })?;
        let _lock = self.store.acquire_apply_lock(relative)?;
        let (key, source) = resolve_source(self.store.workspace(), Path::new(relative))?;
        let before_fingerprint = fingerprint(&source)?;
        if before_fingerprint.blake3 != staged.expected_source_hash {
            return Err(DotallError::SourceHashMismatch {
                path: Path::new(relative).to_path_buf(),
                expected: staged.expected_source_hash,
                actual: before_fingerprint.blake3,
            });
        }

        let before_bytes =
            fs::read(&source).map_err(|source_error| DotallError::io(&source, source_error))?;
        let snapshot_ref = self.store.write_snapshot(relative, &before_bytes)?;
        let loaded = self.model(relative)?;
        let patched = if let Some(snapshot_ref) = restore_snapshot_ref(&staged.preview) {
            let bytes = self.store.read_snapshot(relative, snapshot_ref)?;
            crate::registry::PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            }
        } else {
            loaded.handler.apply_edit(&source, &staged.preview)?
        };
        let actual_output_hash = blake3::hash(&patched.bytes).to_hex().to_string();
        if actual_output_hash != patched.after_source_hash {
            return Err(DotallError::Format {
                format_id: loaded.handler.descriptor().id,
                path: source,
                message: "writer returned bytes whose hash does not match after_source_hash".into(),
            });
        }
        let journal = ApplyJournal {
            tx_id,
            before_source_hash: before_fingerprint.blake3,
            after_source_hash: patched.after_source_hash,
            snapshot_ref,
            committed: false,
        };
        self.store.write_journal(relative, tx_id, &journal)?;
        self.store.replace_source(relative, &patched.bytes)?;
        if interrupt_after_replace {
            return Ok(AppliedEdit {
                tx_id,
                version: 0,
                before_source_hash: journal.before_source_hash,
                after_source_hash: journal.after_source_hash,
                revert_of: revert_of(&staged.preview),
            });
        }
        let journal = ApplyJournal {
            committed: true,
            ..journal
        };
        self.store.write_journal(relative, tx_id, &journal)?;
        if interrupt_after_commit {
            return Ok(AppliedEdit {
                tx_id,
                version: 0,
                before_source_hash: journal.before_source_hash,
                after_source_hash: journal.after_source_hash,
                revert_of: revert_of(&staged.preview),
            });
        }
        let _ = key;
        self.finalize_apply(relative, &loaded.handler, &staged, &journal)
    }

    fn finalize_apply(
        &mut self,
        relative: &str,
        handler: &Arc<dyn FormatHandler>,
        staged: &StagedEdit,
        journal: &ApplyJournal,
    ) -> Result<AppliedEdit> {
        if let Some(record) = self.history_for_transaction(relative, staged.tx_id)? {
            self.store.remove_staged(relative, staged.tx_id)?;
            self.store.discard_journal(relative, staged.tx_id)?;
            return Ok(applied_from_record(&record));
        }
        self.store.invalidate_cache(relative)?;
        self.store
            .register_source(relative, handler.descriptor().id.clone())?;
        let (_, source) = resolve_source(self.store.workspace(), Path::new(relative))?;
        let model = handler.parse(&source)?;
        self.store.write_model(relative, &model)?;
        let record = HistoryRecord {
            version: 0,
            tx_id: staged.tx_id,
            status: HistoryStatus::Applied,
            timestamp: now_unix_ms()?.to_string(),
            actor: staged.actor.clone(),
            ops: staged.preview.operations.clone(),
            semantic_diff: staged.preview.semantic_diff.clone(),
            dependency_impact: staged.preview.dependency_impact.clone(),
            before_source_hash: journal.before_source_hash.clone(),
            after_source_hash: journal.after_source_hash.clone(),
            snapshot_ref: journal.snapshot_ref.clone(),
            revert_of: revert_of(&staged.preview),
        };
        let version = self.store.append_history(relative, &record)?;
        self.store.remove_staged(relative, staged.tx_id)?;
        self.store.discard_journal(relative, staged.tx_id)?;
        Ok(AppliedEdit {
            tx_id: staged.tx_id,
            version,
            before_source_hash: journal.before_source_hash.clone(),
            after_source_hash: journal.after_source_hash.clone(),
            revert_of: record.revert_of,
        })
    }

    fn history_for_transaction(
        &self,
        relative: &str,
        tx_id: Uuid,
    ) -> Result<Option<HistoryRecord>> {
        let version_count = self
            .store
            .manifest()
            .objects
            .get(relative)
            .map(|object| object.version_count)
            .unwrap_or_default();
        for version in 1..=version_count {
            let record = self.store.get_history(relative, version)?;
            if record.tx_id == tx_id {
                return Ok(Some(record));
            }
        }
        Ok(None)
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

    fn model(&mut self, relative: &str) -> Result<LoadedModel> {
        let (key, source) = resolve_source(self.store.workspace(), Path::new(relative))?;
        let prefix = read_prefix(&source)?;
        let handler = self.registry.detect(Path::new(&key), &prefix)?;
        let descriptor = handler.descriptor();
        let schema = handler.artifact_schema();

        let is_fresh = self.store.manifest().objects.contains_key(&key)
            && self
                .store
                .status()?
                .into_iter()
                .find(|object| object.path == key)
                .is_some_and(|object| {
                    matches!(
                        object.state,
                        ObjectState::FreshFastPath | ObjectState::FreshAfterHash
                    )
                });
        if is_fresh
            && let Some(model) =
                self.store
                    .read_model(&key, &schema, &descriptor.id, &descriptor.version)?
        {
            let source_hash = self.store.manifest().objects[&key]
                .fingerprint
                .blake3
                .clone();
            return Ok(LoadedModel {
                key,
                handler,
                model,
                model_cache_hit: true,
                source_hash,
            });
        }

        self.store
            .register_source(&key, handler.descriptor().id.clone())?;
        let model = handler.parse(&source)?;
        self.store.write_model(&key, &model)?;
        let source_hash = self.store.manifest().objects[&key]
            .fingerprint
            .blake3
            .clone();
        Ok(LoadedModel {
            key,
            handler,
            model,
            model_cache_hit: false,
            source_hash,
        })
    }
}

fn restore_snapshot_ref(edit: &ValidatedEdit) -> Option<&str> {
    let operation = edit.operations.first()?;
    (operation.kind == "restore_snapshot")
        .then(|| operation.payload.get("snapshot_ref")?.as_str())?
}

fn revert_of(edit: &ValidatedEdit) -> Option<u64> {
    let operation = edit.operations.first()?;
    (operation.kind == "restore_snapshot").then(|| operation.payload.get("revert_of")?.as_u64())?
}

fn applied_from_record(record: &HistoryRecord) -> AppliedEdit {
    AppliedEdit {
        tx_id: record.tx_id,
        version: record.version,
        before_source_hash: record.before_source_hash.clone(),
        after_source_hash: record.after_source_hash.clone(),
        revert_of: record.revert_of,
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
