mod atomic;

use std::fs;
use std::io::Write;
use std::path::{Component, Path};

use serde::de::DeserializeOwned;

use crate::error::{DotallError, Result};
use crate::fingerprint::{Freshness, check_freshness, fingerprint};
use crate::manifest::{MANIFEST_SCHEMA_VERSION, Manifest, ObjectMeta, OriginalRef, TrackedObject};
use crate::pipeline::{CachedArtifact, CachedDerived, CachedView};
use crate::read::AccessRecord;
use crate::registry::ArtifactEnvelope;
use crate::status::{ObjectState, ObjectStatus};
use crate::workspace::Workspace;

use atomic::write_json;

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
        let version_count = self
            .manifest
            .objects
            .get(&key)
            .map_or(0, |object| object.version_count);

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
            object_dir.join("state/edits/history/snapshots"),
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

    pub fn read_model(&self, relative_path: &str) -> Result<Option<ArtifactEnvelope>> {
        let (key, object) = self.tracked_source(relative_path)?;
        let path = self.cache_path(&key, "model/model.json");
        let Some(cached) = read_cached_json::<CachedArtifact>(&path)? else {
            return Ok(None);
        };

        if cached.source_hash != object.fingerprint.blake3 {
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

fn resolve_source(workspace: &Workspace, relative: &Path) -> Result<(String, std::path::PathBuf)> {
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
