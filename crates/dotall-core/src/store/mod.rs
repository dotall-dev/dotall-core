mod atomic;

use std::fs;
use std::path::{Component, Path};

use crate::error::{DotallError, Result};
use crate::fingerprint::{check_freshness, fingerprint, Freshness};
use crate::manifest::{
    Manifest, ObjectMeta, OriginalRef, TrackedObject, MANIFEST_SCHEMA_VERSION,
};
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

        self.manifest.objects.insert(
            key.clone(),
            TrackedObject {
                format_id: format_id.clone(),
                fingerprint: fingerprint.clone(),
                version_count,
            },
        );

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
            fs::create_dir_all(&directory)
                .map_err(|source| DotallError::io(&directory, source))?;
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
                relative_path: key,
                source_hash: fingerprint.blake3,
            },
        )?;
        write_json(&self.workspace.manifest_path(), &self.manifest)
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

fn resolve_source(
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
        || relative
            .components()
            .next()
            .is_some_and(|component| component.as_os_str() == ".all")
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
