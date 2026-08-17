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

#[derive(Debug, Serialize)]
pub struct EngineStatus {
    pub path: String,
    pub format_id: String,
    pub state: ObjectState,
    pub source_hash: String,
    pub version_count: u64,
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

    /// Borrow the underlying store for read-only workspace queries such as search.
    pub fn store(&self) -> &DotallStore {
        &self.store
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

    pub fn status(&mut self) -> Result<Vec<EngineStatus>> {
        self.store
            .refresh_fresh_fingerprints()?
            .into_iter()
            .map(|object| {
                let tracked = self
                    .store
                    .manifest()
                    .objects
                    .get(&object.path)
                    .ok_or_else(|| DotallError::InvalidSourcePath {
                        path: Path::new(&object.path).to_path_buf(),
                        reason: "status object is missing from the manifest".into(),
                    })?;
                Ok(EngineStatus {
                    path: object.path,
                    format_id: object.format_id,
                    state: object.state,
                    source_hash: tracked.fingerprint.blake3.clone(),
                    version_count: tracked.version_count,
                })
            })
            .collect()
    }

    pub fn deps(
        &mut self,
        relative: &str,
        selector: &str,
        dependents: bool,
    ) -> Result<serde_json::Value> {
        let LoadedModel {
            key,
            handler,
            model,
            source_hash,
            ..
        } = self.model(relative)?;
        handler.query_dependencies(
            &self.store,
            &key,
            &model,
            &source_hash,
            selector,
            dependents,
        )
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

        let (_, source) = resolve_source(self.store.workspace(), Path::new(relative))?;
        let preview = loaded.handler.validate_edit_with_source(
            &source,
            &loaded.model,
            &request.operations,
        )?;
        let staged = StagedEdit {
            tx_id: request.transaction_id,
            operations: request.operations.clone(),
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

    /// Applies staged edits in deterministic order, rebasing stale edits as needed.
    ///
    /// Applies are durable one at a time. If rebasing or applying an edit fails,
    /// prior edits remain committed and the returned error identifies both the
    /// failed transaction and the committed [`AppliedEdit`] records.
    pub fn apply_all(&mut self, relative: &str) -> Result<Vec<AppliedEdit>> {
        let mut staged = self.store.list_staged(relative)?;
        staged.sort_by(|left, right| {
            left.staged_at
                .cmp(&right.staged_at)
                .then_with(|| left.tx_id.cmp(&right.tx_id))
        });

        let mut applied = Vec::with_capacity(staged.len());
        for mut edit in staged {
            let result = (|| {
                let (_, source) = resolve_source(self.store.workspace(), Path::new(relative))?;
                let current_hash = fingerprint(&source)?.blake3;
                if edit.expected_source_hash != current_hash {
                    if is_snapshot_restore(&edit) {
                        edit.expected_source_hash = current_hash;
                    } else {
                        let loaded = self.model(relative)?;
                        let operations = edit.effective_operations();
                        let preview = loaded.handler.validate_edit_with_source(
                            &source,
                            &loaded.model,
                            &operations,
                        )?;
                        edit.expected_source_hash = loaded.source_hash;
                        edit.operations = operations;
                        edit.preview = preview;
                    }
                    self.store.replace_staged(relative, &edit)?;
                }
                self.apply(relative, edit.tx_id)
            })();

            match result {
                Ok(edit) => applied.push(edit),
                Err(source) => {
                    return Err(DotallError::ApplyAllPartial {
                        path: Path::new(relative).to_path_buf(),
                        applied,
                        failed_tx: edit.tx_id,
                        source: Box::new(source),
                    });
                }
            }
        }
        Ok(applied)
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
        let loaded = self.model(relative)?;
        self.decode_snapshot(relative, &loaded.handler, &record.snapshot_ref)?;
        let staged = StagedEdit {
            tx_id,
            operations: vec![SemanticOperation {
                kind: "restore_snapshot".into(),
                payload: serde_json::json!({
                    "snapshot_ref": record.snapshot_ref,
                    "revert_of": version,
                }),
            }],
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
        if !self.store.manifest().objects.contains_key(relative) {
            return Ok(0);
        }
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
                let prefix = read_prefix(&source)?;
                let handler = self.registry.detect(&source, &prefix)?;
                self.finalize_apply(relative, &handler, &staged, &journal)?;
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
        self.recover(relative)?;
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
        let loaded = self.model(relative)?;
        let encoded = loaded.handler.encode_snapshot(&before_bytes)?;
        let decoded = loaded.handler.decode_snapshot(&encoded)?;
        if decoded != before_bytes || encoded.package_hash != before_fingerprint.blake3 {
            return Err(DotallError::Format {
                format_id: loaded.handler.descriptor().id,
                path: source.clone(),
                message: "snapshot encoding did not preserve the exact source bytes".into(),
            });
        }
        let snapshot_ref = self.store.write_encoded_snapshot(relative, &encoded)?;
        let patched = if let Some(snapshot_ref) = restore_snapshot_ref(&staged.preview) {
            let bytes = self.decode_snapshot(relative, &loaded.handler, snapshot_ref)?;
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

    fn decode_snapshot(
        &self,
        relative: &str,
        handler: &Arc<dyn FormatHandler>,
        snapshot_ref: &str,
    ) -> Result<Vec<u8>> {
        let encoded = self.store.read_encoded_snapshot(relative, snapshot_ref)?;
        if encoded.format_id != handler.descriptor().id {
            return Err(DotallError::Format {
                format_id: handler.descriptor().id,
                path: Path::new(relative).to_path_buf(),
                message: format!(
                    "snapshot format {} does not match tracked source format",
                    encoded.format_id
                ),
            });
        }
        let bytes = handler.decode_snapshot(&encoded)?;
        let actual_hash = blake3::hash(&bytes).to_hex().to_string();
        if actual_hash != snapshot_ref || actual_hash != encoded.package_hash {
            return Err(DotallError::Format {
                format_id: encoded.format_id,
                path: Path::new(relative).to_path_buf(),
                message: "decoded snapshot hash does not match snapshot_ref".into(),
            });
        }
        Ok(bytes)
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
            apply_budget(&response.content, request.max_tokens, offset)?;
        response.estimated_tokens = content.chars().count().div_ceil(4);
        response.content = content;
        response.truncated = truncated;
        response.continuation = continuation;
        Ok(response)
    }

    fn model(&mut self, relative: &str) -> Result<LoadedModel> {
        self.recover(relative)?;
        let (key, source) = resolve_source(self.store.workspace(), Path::new(relative))?;
        let prefix = read_prefix(&source)?;
        let handler = self.registry.detect(&source, &prefix)?;
        let descriptor = handler.descriptor();
        let schema = handler.artifact_schema();

        let is_fresh = self.store.manifest().objects.contains_key(&key)
            && self
                .store
                .refresh_fresh_fingerprints()?
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

fn is_snapshot_restore(edit: &StagedEdit) -> bool {
    edit.preview.schema_id == "dotall.restore-snapshot"
        || edit
            .effective_operations()
            .iter()
            .any(|operation| operation.kind == "restore_snapshot")
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

#[cfg(test)]
mod apply_all_tests {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tempfile::tempdir;
    use uuid::Uuid;

    use crate::registry::{
        ArtifactSchema, Capability, DetectionProbe, DetectionScore, FormatDescriptor,
        FormatHandler, FormatRegistry, Inspection, PatchedOutput, ReadRequest,
    };
    use crate::{
        Actor, ActorKind, ArtifactEnvelope, DependencyImpact, DotallStore, EditRequest, Engine,
        ReadResponse, Result, SemanticChange, SemanticOperation, StagedEdit, ValidatedEdit,
    };

    #[test]
    fn apply_all_rebases_legacy_staged_edits_without_top_level_operations() {
        let mut fixture = Fixture::new();
        let hash = fixture.source_hash();
        fixture
            .engine
            .edit(
                "sample.stub",
                &append_request(hash.clone(), "-first", tx(1)),
            )
            .expect("stage first edit");
        fixture
            .engine
            .edit("sample.stub", &append_request(hash, "-second", tx(2)))
            .expect("stage second edit");

        for id in [tx(1), tx(2)] {
            let path = fixture.staged_path(id);
            let staged = fixture
                .engine
                .staged("sample.stub")
                .expect("list staged")
                .into_iter()
                .find(|edit| edit.tx_id == id)
                .expect("staged edit");
            write_legacy_staged_json(&path, &staged);
            let legacy_json = fs::read_to_string(&path).expect("read legacy staged JSON");
            let legacy: StagedEdit =
                serde_json::from_str(&legacy_json).expect("deserialize legacy staged edit");
            assert!(
                legacy.operations.is_empty(),
                "legacy staged JSON should deserialize with empty top-level operations"
            );
            assert_eq!(legacy.effective_operations(), staged.preview.operations);
        }

        let applied = fixture
            .engine
            .apply_all("sample.stub")
            .expect("apply all should rebase legacy staged edits");

        assert_eq!(
            applied.iter().map(|edit| edit.tx_id).collect::<Vec<_>>(),
            vec![tx(1), tx(2)]
        );
        assert_eq!(
            fs::read(fixture.source()).expect("source"),
            b"initial-first-second"
        );
    }

    fn write_legacy_staged_json(path: &Path, staged: &StagedEdit) {
        let mut value =
            serde_json::to_value(staged).expect("serialize staged edit for legacy rewrite");
        value
            .as_object_mut()
            .expect("staged edit JSON object")
            .remove("operations");
        fs::write(
            path,
            serde_json::to_string_pretty(&value).expect("legacy staged JSON"),
        )
        .expect("write legacy staged edit");
    }

    struct Fixture {
        _workspace: tempfile::TempDir,
        engine: Engine,
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let workspace = tempdir().expect("workspace");
            let root = workspace.path().to_path_buf();
            fs::write(root.join("sample.stub"), b"initial").expect("source");
            let store = DotallStore::init(&root).expect("store");
            let parse_count = Arc::new(AtomicUsize::new(0));
            let mut registry = FormatRegistry::default();
            registry.register(Arc::new(StubFormat {
                parse_count: Arc::clone(&parse_count),
            }));
            Self {
                _workspace: workspace,
                engine: Engine::new(store, registry),
                root,
            }
        }

        fn source(&self) -> std::path::PathBuf {
            self.root.join("sample.stub")
        }

        fn staged_path(&self, tx_id: Uuid) -> std::path::PathBuf {
            self.root.join(format!(
                ".all/objects/sample.stub/state/edits/staging/{tx_id}.json"
            ))
        }

        fn source_hash(&mut self) -> String {
            self.engine
                .load_model("sample.stub")
                .expect("load model")
                .source_hash
        }
    }

    fn tx(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn append_request(
        expected_source_hash: String,
        suffix: &str,
        transaction_id: Uuid,
    ) -> EditRequest {
        EditRequest {
            transaction_id,
            expected_source_hash,
            actor: Actor {
                kind: ActorKind::Cli,
                id: Some("test".into()),
            },
            operations: vec![SemanticOperation {
                kind: "append".into(),
                payload: serde_json::json!({ "suffix": suffix }),
            }],
        }
    }

    struct StubFormat {
        parse_count: Arc<AtomicUsize>,
    }

    impl FormatHandler for StubFormat {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor {
                id: "stub".into(),
                version: "1".into(),
                capabilities: vec![Capability::Inspect, Capability::ReadFull],
                edit_capabilities: Vec::new(),
            }
        }

        fn artifact_schema(&self) -> ArtifactSchema {
            ArtifactSchema {
                format_id: "stub".into(),
                schema_id: "stub.document".into(),
                schema_version: 1,
            }
        }

        fn detect(&self, probe: &DetectionProbe<'_>) -> DetectionScore {
            DetectionScore((probe.path.extension() == Some("stub".as_ref())) as u16)
        }

        fn parse(&self, source: &Path) -> Result<ArtifactEnvelope> {
            self.parse_count.fetch_add(1, Ordering::SeqCst);
            let content = String::from_utf8(fs::read(source).expect("test source"))
                .expect("test source UTF-8");
            Ok(ArtifactEnvelope {
                format_id: "stub".into(),
                schema_id: "stub.document".into(),
                schema_version: 1,
                payload: serde_json::json!({ "content": content }),
            })
        }

        fn inspect(&self, _model: &ArtifactEnvelope) -> Result<Inspection> {
            unreachable!("apply_all legacy rebase test only")
        }

        fn read(&self, _model: &ArtifactEnvelope, _request: &ReadRequest) -> Result<ReadResponse> {
            unreachable!("apply_all legacy rebase test only")
        }

        fn validate_edit(
            &self,
            model: &ArtifactEnvelope,
            operations: &[SemanticOperation],
        ) -> Result<ValidatedEdit> {
            let operation = &operations[0];
            let content = model.payload["content"].as_str().expect("content");
            let value = match operation.kind.as_str() {
                "append" => format!(
                    "{content}{}",
                    operation.payload["suffix"].as_str().expect("suffix")
                ),
                other => panic!("unsupported test operation: {other}"),
            };
            Ok(ValidatedEdit {
                format_id: "stub".into(),
                schema_id: "stub.edits".into(),
                schema_version: 1,
                operations: operations.to_vec(),
                semantic_diff: vec![SemanticChange {
                    target: "content".into(),
                    element_id: "content".into(),
                    change: operation.kind.clone(),
                    before: Some(content.into()),
                    after: Some(value),
                }],
                dependency_impact: DependencyImpact {
                    forward: Vec::new(),
                    notes: Vec::new(),
                },
            })
        }

        fn apply_edit(&self, source: &Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
            let operation = &edit.operations[0];
            let bytes = match operation.kind.as_str() {
                "append" => {
                    let mut bytes = fs::read(source).expect("test source");
                    bytes.extend_from_slice(
                        operation.payload["suffix"]
                            .as_str()
                            .expect("suffix")
                            .as_bytes(),
                    );
                    bytes
                }
                other => panic!("unsupported test operation: {other}"),
            };
            Ok(PatchedOutput {
                after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
                bytes,
            })
        }
    }
}
