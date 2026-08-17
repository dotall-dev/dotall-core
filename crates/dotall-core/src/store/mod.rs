mod atomic;

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::error::{DotallError, Result};
use crate::fingerprint::{Freshness, check_freshness, fingerprint};
use crate::history::{
    ApplyLock, CancelAudit, CancelStatus, HistoryRecord, HistorySummary, StagedEdit,
    history_version_file_name,
};
use crate::manifest::{MANIFEST_SCHEMA_VERSION, Manifest, ObjectMeta, OriginalRef, TrackedObject};
use crate::pipeline::{CachedArtifact, CachedDerived, CachedView, DerivationRecipe};
use crate::read::AccessRecord;
use crate::registry::{ArtifactEnvelope, ArtifactSchema, EncodedSnapshot, SnapshotPart};
use crate::status::{ObjectState, ObjectStatus};
use crate::workspace::Workspace;

use atomic::{write_bytes, write_json, write_json_new};

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
struct StoredSnapshotManifest {
    package_hash: String,
    format_id: String,
    manifest: serde_json::Value,
    part_hashes: Vec<String>,
}

#[derive(Debug)]
pub struct DotallStore {
    workspace: Workspace,
    manifest: Manifest,
}

impl DotallStore {
    pub fn init(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        fs::create_dir_all(root).map_err(|source| DotallError::io(root, source))?;
        let workspace = Workspace::at(root)?;
        let all_dir = workspace.all_dir();
        if all_dir.exists() && !all_dir.is_dir() {
            return Err(DotallError::InvalidWorkspacePath(all_dir));
        }
        fs::create_dir_all(workspace.objects_dir())
            .map_err(|source| DotallError::io(workspace.objects_dir(), source))?;

        let manifest = if workspace.manifest_path().is_file() {
            load_manifest(&workspace)?
        } else {
            let manifest = Manifest::default();
            write_json(&workspace.manifest_path(), &manifest)?;
            manifest
        };

        Ok(Self {
            workspace,
            manifest,
        })
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let workspace = Workspace::discover(root)?;
        let manifest = load_manifest(&workspace)?;
        Ok(Self {
            workspace,
            manifest,
        })
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn register_source(
        &mut self,
        relative_path: impl AsRef<Path>,
        format_id: impl Into<String>,
    ) -> Result<()> {
        let (key, source) = resolve_source(&self.workspace, relative_path.as_ref())?;
        let fingerprint = fingerprint(&source)?;
        let format_id = format_id.into();
        let disk_version_count = load_manifest(&self.workspace)
            .ok()
            .and_then(|manifest| {
                manifest
                    .objects
                    .get(&key)
                    .map(|object| object.version_count)
            })
            .unwrap_or(0);
        let memory_version_count = self
            .manifest
            .objects
            .get(&key)
            .map_or(0, |object| object.version_count);
        let version_count = memory_version_count.max(disk_version_count);

        let tracked = TrackedObject {
            format_id: format_id.clone(),
            fingerprint: fingerprint.clone(),
            version_count,
        };

        let object_dir = self.workspace.objects_dir().join(&key);
        for directory in [
            object_dir.join("cache/model"),
            object_dir.join("cache/derived"),
            object_dir.join("cache/views"),
            object_dir.join("state/access"),
            object_dir.join("state/transactions"),
            object_dir.join("state/edits/staging"),
            object_dir.join("state/edits/history/snapshots/manifests"),
            object_dir.join("state/edits/history/snapshots/parts"),
        ] {
            fs::create_dir_all(&directory).map_err(|source| DotallError::io(&directory, source))?;
        }

        write_json(
            &object_dir.join("meta.json"),
            &ObjectMeta {
                schema_version: MANIFEST_SCHEMA_VERSION,
                format_id,
                fingerprint: fingerprint.clone(),
            },
        )?;
        write_json(
            &object_dir.join("original.ref"),
            &OriginalRef {
                relative_path: key.clone(),
                source_hash: fingerprint.blake3,
            },
        )?;

        let mut next_manifest = self.manifest.clone();
        next_manifest.objects.insert(key, tracked);
        write_json(&self.workspace.manifest_path(), &next_manifest)?;
        self.manifest = next_manifest;
        Ok(())
    }

    /// Persist fingerprints after a metadata-only change so the next status
    /// check can use the mtime/size fast path instead of rehashing.
    pub fn refresh_fresh_fingerprints(&mut self) -> Result<Vec<ObjectStatus>> {
        let mut next_manifest = self.manifest.clone();
        let mut wrote = false;
        let mut statuses = Vec::new();

        for (path, object) in &self.manifest.objects {
            let source = self.workspace.root().join(path);
            let (state, updated) = if !source.is_file() {
                (ObjectState::Missing, None)
            } else {
                match check_freshness(&source, &object.fingerprint)? {
                    Freshness::FreshFastPath => (ObjectState::FreshFastPath, None),
                    Freshness::FreshAfterHash(fingerprint) => {
                        (ObjectState::FreshAfterHash, Some(fingerprint))
                    }
                    Freshness::Stale(_) => (ObjectState::Stale, None),
                }
            };
            if let Some(fingerprint) = updated {
                if let Some(tracked) = next_manifest.objects.get_mut(path) {
                    tracked.fingerprint = fingerprint.clone();
                }
                write_json(
                    &self.workspace.objects_dir().join(path).join("meta.json"),
                    &ObjectMeta {
                        schema_version: MANIFEST_SCHEMA_VERSION,
                        format_id: object.format_id.clone(),
                        fingerprint,
                    },
                )?;
                wrote = true;
            }
            statuses.push(ObjectStatus {
                path: path.clone(),
                format_id: object.format_id.clone(),
                state,
            });
        }

        if wrote {
            write_json(&self.workspace.manifest_path(), &next_manifest)?;
            self.manifest = next_manifest;
        }
        Ok(statuses)
    }

    pub fn status(&self) -> Result<Vec<ObjectStatus>> {
        self.manifest
            .objects
            .iter()
            .map(|(path, object)| {
                let source = self.workspace.root().join(path);
                let state = if !source.is_file() {
                    ObjectState::Missing
                } else {
                    match check_freshness(&source, &object.fingerprint)? {
                        Freshness::FreshFastPath => ObjectState::FreshFastPath,
                        Freshness::FreshAfterHash(_) => ObjectState::FreshAfterHash,
                        Freshness::Stale(_) => ObjectState::Stale,
                    }
                };
                Ok(ObjectStatus {
                    path: path.clone(),
                    format_id: object.format_id.clone(),
                    state,
                })
            })
            .collect()
    }

    pub fn write_model(&self, relative_path: &str, artifact: &ArtifactEnvelope) -> Result<()> {
        let (key, object) = self.tracked_source(relative_path)?;
        let cached = CachedArtifact {
            source_hash: object.fingerprint.blake3.clone(),
            producer_id: artifact.format_id.clone(),
            producer_version: artifact.schema_version.to_string(),
            artifact: artifact.clone(),
        };

        write_json(&self.cache_path(&key, "model/model.json"), &cached)
    }

    pub fn read_model(
        &self,
        relative_path: &str,
        expected_schema: &ArtifactSchema,
        expected_producer_id: &str,
        expected_producer_version: &str,
    ) -> Result<Option<ArtifactEnvelope>> {
        let (key, object) = self.tracked_source(relative_path)?;
        let path = self.cache_path(&key, "model/model.json");
        let Some(cached) = read_cached_json::<CachedArtifact>(&path)? else {
            return Ok(None);
        };

        if cached.source_hash != object.fingerprint.blake3
            || cached.producer_id != expected_producer_id
            || cached.producer_version != expected_producer_version
            || cached.artifact.format_id != expected_schema.format_id
            || cached.artifact.schema_id != expected_schema.schema_id
            || cached.artifact.schema_version != expected_schema.schema_version
        {
            return Ok(None);
        }
        Ok(Some(cached.artifact))
    }

    pub fn write_view(&self, relative_path: &str, view: &CachedView) -> Result<()> {
        let (key, _) = self.tracked_source(relative_path)?;
        let request_hash = cache_name(&view.request_hash, "view request hash")?;
        write_json(
            &self.cache_path(&key, &format!("views/{request_hash}.json")),
            view,
        )
    }

    pub fn read_view(&self, relative_path: &str, request_hash: &str) -> Result<Option<CachedView>> {
        let (key, object) = self.tracked_source(relative_path)?;
        let request_hash = cache_name(request_hash, "view request hash")?;
        let path = self.cache_path(&key, &format!("views/{request_hash}.json"));
        let Some(cached) = read_cached_json::<CachedView>(&path)? else {
            return Ok(None);
        };

        Ok(
            (cached.source_hash == object.fingerprint.blake3
                && cached.request_hash == request_hash)
                .then_some(cached),
        )
    }

    pub fn write_derived(
        &self,
        relative_path: &str,
        name: &str,
        derived: &CachedDerived,
    ) -> Result<()> {
        let (key, _) = self.tracked_source(relative_path)?;
        let name = cache_name(name, "derived artifact name")?;
        write_json(
            &self.cache_path(&key, &format!("derived/{name}.json")),
            derived,
        )
    }

    pub fn read_derived(&self, relative_path: &str, name: &str) -> Result<Option<CachedDerived>> {
        let (key, object) = self.tracked_source(relative_path)?;
        let name = cache_name(name, "derived artifact name")?;
        let path = self.cache_path(&key, &format!("derived/{name}.json"));
        let Some(cached) = read_cached_json::<CachedDerived>(&path)? else {
            return Ok(None);
        };

        Ok((cached.source_hash == object.fingerprint.blake3).then_some(cached))
    }

    /// Stores a derived artifact under the deterministic key for `recipe`.
    ///
    /// The source hash is stamped from the tracked manifest so a caller cannot
    /// accidentally persist a stale hash.
    pub fn write_derived_for_recipe(
        &self,
        relative_path: &str,
        recipe: &DerivationRecipe,
        derived: &CachedDerived,
    ) -> Result<()> {
        let (key, object) = self.tracked_source(relative_path)?;
        let recipe_key = recipe.key()?;
        let derived = CachedDerived {
            source_hash: object.fingerprint.blake3.clone(),
            ..derived.clone()
        };
        write_json(
            &self.cache_path(&key, &format!("derived/{recipe_key}.json")),
            &derived,
        )
    }

    /// Reads a derived artifact only when its recipe and model identity match.
    pub fn read_derived_for_recipe(
        &self,
        relative_path: &str,
        recipe: &DerivationRecipe,
        model_schema_id: &str,
        model_schema_version: u32,
    ) -> Result<Option<CachedDerived>> {
        let (key, object) = self.tracked_source(relative_path)?;
        let recipe_key = recipe.key()?;
        let path = self.cache_path(&key, &format!("derived/{recipe_key}.json"));
        let Some(cached) = read_cached_json::<CachedDerived>(&path)? else {
            return Ok(None);
        };

        Ok((recipe.source_hash == object.fingerprint.blake3
            && cached.source_hash == object.fingerprint.blake3
            && cached.model_schema_id == model_schema_id
            && cached.model_schema_version == model_schema_version
            && cached.processor_id == recipe.processor_id
            && cached.processor_version == recipe.processor_version)
            .then_some(cached))
    }

    pub fn append_access(&self, relative_path: &str, record: &AccessRecord<'_>) -> Result<()> {
        let (key, _) = self.tracked_source(relative_path)?;
        let path = self
            .workspace
            .objects_dir()
            .join(key)
            .join("state/access/log.jsonl");
        let mut bytes =
            serde_json::to_vec(record).map_err(|source| DotallError::Serialization {
                context: "access record".into(),
                source,
            })?;
        bytes.push(b'\n');

        let mut file = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .map_err(|source| DotallError::io(&path, source))?;
        file.write_all(&bytes)
            .map_err(|source| DotallError::io(&path, source))?;
        file.sync_data()
            .map_err(|source| DotallError::io(&path, source))
    }

    /// Persists a staged edit under `state/edits/staging/<tx_id>.json`.
    ///
    /// Retrying with the same transaction id and payload is idempotent. A conflicting
    /// payload for the same transaction id returns [`DotallError::StagedConflict`].
    pub fn stage_edit(&self, relative_path: &str, staged: &StagedEdit) -> Result<()> {
        let (key, _) = self.tracked_source(relative_path)?;
        let path = self.staging_path(&key, staged.tx_id);
        if path.is_file() {
            let existing: StagedEdit = read_required_json(&path, "staged edit")?;
            if existing.same_payload(staged) {
                return Ok(());
            }
            return Err(DotallError::StagedConflict {
                path: Path::new(relative_path).to_path_buf(),
                tx_id: staged.tx_id.to_string(),
            });
        }
        write_json(&path, staged)
    }

    pub fn read_staged(&self, relative_path: &str, tx_id: Uuid) -> Result<Option<StagedEdit>> {
        let (key, _) = self.tracked_source(relative_path)?;
        read_cached_json(&self.staging_path(&key, tx_id))
    }

    /// Replaces a previously staged edit after its preview has been rebased.
    pub fn replace_staged(&self, relative_path: &str, staged: &StagedEdit) -> Result<()> {
        let (key, _) = self.tracked_source(relative_path)?;
        let path = self.staging_path(&key, staged.tx_id);
        if !path.is_file() {
            return Err(DotallError::StagedMissing {
                path: Path::new(relative_path).to_path_buf(),
                tx_id: staged.tx_id.to_string(),
            });
        }
        write_json(&path, staged)
    }

    pub fn list_staged(&self, relative_path: &str) -> Result<Vec<StagedEdit>> {
        let (key, _) = self.tracked_source(relative_path)?;
        let directory = self.staging_dir(&key);
        if !directory.is_dir() {
            return Ok(Vec::new());
        }

        let mut staged = Vec::new();
        for entry in
            fs::read_dir(&directory).map_err(|source| DotallError::io(&directory, source))?
        {
            let entry = entry.map_err(|source| DotallError::io(&directory, source))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            staged.push(read_required_json(&path, "staged edit")?);
        }
        Ok(staged)
    }

    pub fn discard_staged(&self, relative_path: &str, tx_id: Uuid) -> Result<()> {
        let (key, _) = self.tracked_source(relative_path)?;
        let path = self.staging_path(&key, tx_id);
        let Some(staged) = read_cached_json::<StagedEdit>(&path)? else {
            return Err(DotallError::StagedMissing {
                path: Path::new(relative_path).to_path_buf(),
                tx_id: tx_id.to_string(),
            });
        };

        let audit = CancelAudit {
            tx_id,
            status: CancelStatus::Cancelled,
            timestamp: crate::read::now_unix_ms()?.to_string(),
            actor: staged.actor,
            reason: "discard".into(),
        };
        fs::remove_file(&path).map_err(|source| DotallError::io(&path, source))?;
        write_json(&self.cancel_path(&key, tx_id), &audit)
    }

    /// Removes a staged edit file without writing a cancel audit.
    pub(crate) fn remove_staged(&self, relative_path: &str, tx_id: Uuid) -> Result<()> {
        let (key, _) = self.tracked_source(relative_path)?;
        let path = self.staging_path(&key, tx_id);
        if path.is_file() {
            fs::remove_file(&path).map_err(|source| DotallError::io(&path, source))?;
        }
        let cancel_path = self.cancel_path(&key, tx_id);
        if cancel_path.is_file() {
            fs::remove_file(&cancel_path)
                .map_err(|source| DotallError::io(&cancel_path, source))?;
        }
        Ok(())
    }

    /// Acquires the per-object apply lock at `state/edits/.apply.lock`.
    pub fn acquire_apply_lock(&self, relative_path: &str) -> Result<ApplyLock> {
        let (key, _) = self.tracked_source(relative_path)?;
        ApplyLock::acquire(&self.apply_lock_path(&key))
    }

    /// Persists a format-owned snapshot manifest and its content-addressed parts.
    pub fn write_encoded_snapshot(
        &self,
        relative_path: &str,
        encoded: &EncodedSnapshot,
    ) -> Result<String> {
        let (key, _) = self.tracked_source(relative_path)?;
        for part in &encoded.parts {
            let actual_hash = blake3::hash(&part.bytes).to_hex().to_string();
            if actual_hash != part.hash {
                return Err(DotallError::InvalidSourcePath {
                    path: self.snapshot_part_path(&key, &part.hash),
                    reason: "snapshot part hash does not match its bytes".to_owned(),
                });
            }
            self.put_snapshot_part(&key, part)?;
        }
        let stored = StoredSnapshotManifest {
            package_hash: encoded.package_hash.clone(),
            format_id: encoded.format_id.clone(),
            manifest: encoded.manifest.clone(),
            part_hashes: encoded.parts.iter().map(|part| part.hash.clone()).collect(),
        };
        self.put_snapshot_manifest(&key, &stored)?;
        Ok(encoded.package_hash.clone())
    }

    pub fn read_encoded_snapshot(
        &self,
        relative_path: &str,
        package_hash: &str,
    ) -> Result<EncodedSnapshot> {
        let (key, _) = self.tracked_source(relative_path)?;
        let stored = self.get_snapshot_manifest(&key, relative_path, package_hash)?;
        let parts = stored
            .part_hashes
            .iter()
            .map(|hash| {
                self.get_snapshot_part(&key, relative_path, hash)
                    .map(|bytes| SnapshotPart {
                        hash: hash.clone(),
                        bytes,
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(EncodedSnapshot {
            package_hash: stored.package_hash,
            format_id: stored.format_id,
            manifest: stored.manifest,
            parts,
        })
    }

    /// Replaces a tracked source atomically on its own filesystem.
    pub fn replace_source(&self, relative_path: &str, bytes: &[u8]) -> Result<()> {
        let (_, source) = resolve_source(&self.workspace, Path::new(relative_path))?;
        write_bytes(&source, bytes)
    }

    /// Removes regenerable model, derived, and view artifacts after a source mutation.
    pub fn invalidate_cache(&self, relative_path: &str) -> Result<()> {
        let (key, _) = self.tracked_source(relative_path)?;
        let cache = self.object_dir(&key).join("cache");
        if cache.exists() {
            fs::remove_dir_all(&cache).map_err(|source| DotallError::io(&cache, source))?;
        }
        for directory in ["model", "derived", "views"] {
            let path = cache.join(directory);
            fs::create_dir_all(&path).map_err(|source| DotallError::io(&path, source))?;
        }
        Ok(())
    }

    pub fn write_journal<T: Serialize>(
        &self,
        relative_path: &str,
        tx_id: Uuid,
        record: &T,
    ) -> Result<()> {
        let (key, _) = self.tracked_source(relative_path)?;
        write_json(&self.journal_path(&key, tx_id), record)
    }

    pub fn read_journals<T: DeserializeOwned>(&self, relative_path: &str) -> Result<Vec<T>> {
        let (key, _) = self.tracked_source(relative_path)?;
        let directory = self.transaction_dir(&key);
        if !directory.is_dir() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for entry in
            fs::read_dir(&directory).map_err(|source| DotallError::io(&directory, source))?
        {
            let entry = entry.map_err(|source| DotallError::io(&directory, source))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if file_name.ends_with(".cancel.json") {
                continue;
            }
            records.push(read_required_json(&path, "transaction journal")?);
        }
        Ok(records)
    }

    pub fn discard_journal(&self, relative_path: &str, tx_id: Uuid) -> Result<()> {
        let (key, _) = self.tracked_source(relative_path)?;
        let path = self.journal_path(&key, tx_id);
        if path.is_file() {
            fs::remove_file(&path).map_err(|source| DotallError::io(&path, source))?;
        }
        Ok(())
    }

    /// Appends one forensic history version under `state/edits/history/vNNN.json`.
    ///
    /// Versions are sequential starting at 1. The stored record's `version` field is
    /// assigned from the manifest; callers may pass `0` as a placeholder.
    ///
    /// Callers must hold [`ApplyLock`] for the source so concurrent appends cannot
    /// race on version allocation or history file creation.
    pub fn append_history(&mut self, relative_path: &str, record: &HistoryRecord) -> Result<u64> {
        let (key, object) = self.tracked_source(relative_path)?;
        let next_version = object.version_count.saturating_add(1);
        let history_dir = self.history_dir(&key);
        fs::create_dir_all(&history_dir).map_err(|source| DotallError::io(&history_dir, source))?;

        let path = self.history_version_path(&key, next_version);
        let persisted = HistoryRecord {
            version: next_version,
            ..record.clone()
        };
        if let Err(err) = write_json_new(&path, &persisted) {
            return Err(map_history_version_exists(relative_path, next_version, err));
        }

        let mut next_manifest = self.manifest.clone();
        let tracked =
            next_manifest
                .objects
                .get_mut(&key)
                .ok_or_else(|| DotallError::InvalidSourcePath {
                    path: Path::new(relative_path).to_path_buf(),
                    reason: "source is not tracked".to_owned(),
                })?;
        tracked.version_count = next_version;
        if let Err(err) = write_json(&self.workspace.manifest_path(), &next_manifest) {
            let _ = fs::remove_file(&path);
            return Err(err);
        }
        self.manifest = next_manifest;
        Ok(next_version)
    }

    /// Returns compact agent-facing history summaries in ascending version order.
    pub fn list_history(&self, relative_path: &str) -> Result<Vec<HistorySummary>> {
        let (key, _) = self.tracked_source(relative_path)?;
        let directory = self.history_dir(&key);
        if !directory.is_dir() {
            return Ok(Vec::new());
        }

        let mut summaries = Vec::new();
        for entry in
            fs::read_dir(&directory).map_err(|source| DotallError::io(&directory, source))?
        {
            let entry = entry.map_err(|source| DotallError::io(&directory, source))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !file_name.starts_with('v') {
                continue;
            }
            let record: HistoryRecord = read_required_json(&path, "history record")?;
            summaries.push(record.summary());
        }

        summaries.sort_by_key(|summary| summary.version);
        Ok(summaries)
    }

    /// Loads one full forensic history record by version number.
    pub fn get_history(&self, relative_path: &str, version: u64) -> Result<HistoryRecord> {
        let (key, _) = self.tracked_source(relative_path)?;
        let path = self.history_version_path(&key, version);
        read_cached_json(&path)?.ok_or_else(|| DotallError::HistoryVersionMissing {
            path: Path::new(relative_path).to_path_buf(),
            version,
        })
    }

    fn tracked_source(&self, relative_path: &str) -> Result<(String, &TrackedObject)> {
        let (key, _) = resolve_source(&self.workspace, Path::new(relative_path))?;
        let object =
            self.manifest
                .objects
                .get(&key)
                .ok_or_else(|| DotallError::InvalidSourcePath {
                    path: Path::new(relative_path).to_path_buf(),
                    reason: "source is not tracked".to_owned(),
                })?;
        Ok((key, object))
    }

    fn cache_path(&self, key: &str, suffix: &str) -> std::path::PathBuf {
        self.workspace
            .objects_dir()
            .join(key)
            .join("cache")
            .join(suffix)
    }

    fn object_dir(&self, key: &str) -> PathBuf {
        self.workspace.objects_dir().join(key)
    }

    fn staging_dir(&self, key: &str) -> PathBuf {
        self.object_dir(key).join("state/edits/staging")
    }

    fn staging_path(&self, key: &str, tx_id: Uuid) -> PathBuf {
        self.staging_dir(key).join(format!("{tx_id}.json"))
    }

    fn apply_lock_path(&self, key: &str) -> PathBuf {
        self.object_dir(key).join("state/edits/.apply.lock")
    }

    fn transaction_dir(&self, key: &str) -> PathBuf {
        self.object_dir(key).join("state/transactions")
    }

    fn journal_path(&self, key: &str, tx_id: Uuid) -> PathBuf {
        self.transaction_dir(key).join(format!("{tx_id}.json"))
    }

    fn cancel_path(&self, key: &str, tx_id: Uuid) -> PathBuf {
        self.transaction_dir(key)
            .join(format!("{tx_id}.cancel.json"))
    }

    fn snapshot_manifest_path(&self, key: &str, hash: &str) -> PathBuf {
        self.object_dir(key)
            .join("state/edits/history/snapshots/manifests")
            .join(format!("{hash}.json"))
    }

    fn snapshot_part_path(&self, key: &str, hash: &str) -> PathBuf {
        self.object_dir(key)
            .join("state/edits/history/snapshots/parts")
            .join(hash)
    }

    fn put_snapshot_part(&self, key: &str, part: &SnapshotPart) -> Result<()> {
        let path = self.snapshot_part_path(key, &part.hash);
        if path.is_file() {
            let existing = fs::read(&path).map_err(|source| DotallError::io(&path, source))?;
            if existing == part.bytes {
                return Ok(());
            }
            return Err(DotallError::InvalidSourcePath {
                path,
                reason: "snapshot part path exists with different content".to_owned(),
            });
        }
        write_bytes(&path, &part.bytes)
    }

    fn get_snapshot_part(&self, key: &str, relative_path: &str, hash: &str) -> Result<Vec<u8>> {
        let path = self.snapshot_part_path(key, hash);
        fs::read(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                DotallError::SnapshotMissing {
                    path: Path::new(relative_path).to_path_buf(),
                    hash: hash.to_owned(),
                }
            } else {
                DotallError::io(&path, source)
            }
        })
    }

    fn put_snapshot_manifest(&self, key: &str, manifest: &StoredSnapshotManifest) -> Result<()> {
        let path = self.snapshot_manifest_path(key, &manifest.package_hash);
        if path.is_file() {
            let existing: StoredSnapshotManifest = read_required_json(&path, "snapshot manifest")?;
            if existing == *manifest {
                return Ok(());
            }
            return Err(DotallError::InvalidSourcePath {
                path,
                reason: "snapshot manifest path exists with different content".to_owned(),
            });
        }
        write_json(&path, manifest)
    }

    fn get_snapshot_manifest(
        &self,
        key: &str,
        relative_path: &str,
        hash: &str,
    ) -> Result<StoredSnapshotManifest> {
        let path = self.snapshot_manifest_path(key, hash);
        read_cached_json(&path)?.ok_or_else(|| DotallError::SnapshotMissing {
            path: Path::new(relative_path).to_path_buf(),
            hash: hash.to_owned(),
        })
    }

    fn history_dir(&self, key: &str) -> PathBuf {
        self.object_dir(key).join("state/edits/history")
    }

    fn history_version_path(&self, key: &str, version: u64) -> PathBuf {
        self.history_dir(key)
            .join(history_version_file_name(version))
    }
}

fn load_manifest(workspace: &Workspace) -> Result<Manifest> {
    let path = workspace.manifest_path();
    let bytes = fs::read(&path).map_err(|source| DotallError::io(&path, source))?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|source| DotallError::InvalidManifest {
            path: path.clone(),
            source,
        })?;
    manifest.validate()?;
    Ok(manifest)
}

fn read_cached_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.is_file() {
        return Ok(None);
    }

    let bytes = fs::read(path).map_err(|source| DotallError::io(path, source))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| DotallError::Serialization {
            context: format!("cached artifact at {}", path.display()),
            source,
        })
}

fn read_required_json<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T> {
    read_cached_json(path)?.ok_or_else(|| DotallError::InvalidSourcePath {
        path: path.to_path_buf(),
        reason: format!("missing {label}"),
    })
}

fn cache_name<'a>(name: &'a str, label: &str) -> Result<&'a str> {
    let path = Path::new(name);
    if name.is_empty()
        || name.contains(['/', '\\'])
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(DotallError::InvalidSourcePath {
            path: path.to_path_buf(),
            reason: format!("{label} must be a single file-name component"),
        });
    }
    Ok(name)
}

pub(crate) fn resolve_source(
    workspace: &Workspace,
    relative: &Path,
) -> Result<(String, std::path::PathBuf)> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(DotallError::InvalidSourcePath {
            path: relative.to_path_buf(),
            reason: "path must be relative, remain inside the workspace, and not target .all"
                .to_owned(),
        });
    }

    let key = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>();
    if key.is_empty()
        || key.len()
            != relative
                .components()
                .filter(|component| matches!(component, Component::Normal(_)))
                .count()
    {
        return Err(DotallError::InvalidSourcePath {
            path: relative.to_path_buf(),
            reason: "path contains a non-UTF-8 or unsupported component".to_owned(),
        });
    }
    if key.contains(&".all") {
        return Err(DotallError::InvalidSourcePath {
            path: relative.to_path_buf(),
            reason: "path must be relative, remain inside the workspace, and not target .all"
                .to_owned(),
        });
    }
    let key = key.join("/");
    let source = workspace.root().join(&key);
    if !source.is_file() {
        return Err(DotallError::InvalidSourcePath {
            path: relative.to_path_buf(),
            reason: "source is not a regular file".to_owned(),
        });
    }
    Ok((key, source))
}

fn map_history_version_exists(relative_path: &str, version: u64, err: DotallError) -> DotallError {
    if matches!(
        &err,
        DotallError::Io { source, .. } if source.kind() == std::io::ErrorKind::AlreadyExists
    ) {
        DotallError::HistoryVersionExists {
            path: Path::new(relative_path).to_path_buf(),
            version,
        }
    } else {
        err
    }
}
