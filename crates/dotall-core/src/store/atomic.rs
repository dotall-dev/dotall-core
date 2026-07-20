use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

use serde::Serialize;

use crate::error::{DotallError, Result};

pub(crate) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| DotallError::InvalidWorkspacePath(path.to_path_buf()))?;
    fs::create_dir_all(parent).map_err(|source| DotallError::io(parent, source))?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DotallError::InvalidWorkspacePath(path.to_path_buf()))?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));

    let result = (|| -> Result<()> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| DotallError::io(&temporary, source))?;
        let mut writer = BufWriter::new(file);
        writer
            .write_all(bytes)
            .map_err(|source| DotallError::io(&temporary, source))?;
        writer
            .flush()
            .map_err(|source| DotallError::io(&temporary, source))?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|source| DotallError::io(&temporary, source))?;

        rename_replace(&temporary, path)?;
        sync_parent(parent)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| DotallError::InvalidWorkspacePath(path.to_path_buf()))?;
    fs::create_dir_all(parent).map_err(|source| DotallError::io(parent, source))?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DotallError::InvalidWorkspacePath(path.to_path_buf()))?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));

    let result = (|| -> Result<()> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| DotallError::io(&temporary, source))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value).map_err(|source| {
            DotallError::InvalidManifest {
                path: temporary.clone(),
                source,
            }
        })?;
        writer
            .write_all(b"\n")
            .map_err(|source| DotallError::io(&temporary, source))?;
        writer
            .flush()
            .map_err(|source| DotallError::io(&temporary, source))?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|source| DotallError::io(&temporary, source))?;

        rename_replace(&temporary, path)?;
        sync_parent(parent)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Writes JSON atomically, failing if `path` already exists.
pub(crate) fn write_json_new<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| DotallError::InvalidWorkspacePath(path.to_path_buf()))?;
    fs::create_dir_all(parent).map_err(|source| DotallError::io(parent, source))?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DotallError::InvalidWorkspacePath(path.to_path_buf()))?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));

    let result = (|| -> Result<()> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| DotallError::io(&temporary, source))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value).map_err(|source| {
            DotallError::InvalidManifest {
                path: temporary.clone(),
                source,
            }
        })?;
        writer
            .write_all(b"\n")
            .map_err(|source| DotallError::io(&temporary, source))?;
        writer
            .flush()
            .map_err(|source| DotallError::io(&temporary, source))?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|source| DotallError::io(&temporary, source))?;

        rename_new(&temporary, path)?;
        sync_parent(parent)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn rename_new(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        return Err(DotallError::io(
            to,
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "destination already exists",
            ),
        ));
    }
    fs::rename(from, to).map_err(|source| DotallError::io(to, source))
}

fn rename_replace(from: &Path, to: &Path) -> Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(source) => {
            // On Windows, rename fails when the destination already exists.
            if to.exists() {
                fs::remove_file(to).map_err(|source| DotallError::io(to, source))?;
                fs::rename(from, to).map_err(|source| DotallError::io(to, source))
            } else {
                Err(DotallError::io(to, source))
            }
        }
    }
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<()> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| DotallError::io(parent, source))
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::DotallError;
    use crate::manifest::Manifest;

    use super::{write_json, write_json_new};

    #[test]
    fn writes_complete_json_and_removes_temporary_file() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("manifest.json");

        write_json(&path, &Manifest::default()).expect("write manifest");

        let parsed: Manifest = serde_json::from_slice(&fs::read(&path).expect("read manifest"))
            .expect("parse manifest");
        assert_eq!(parsed, Manifest::default());
        assert!(
            !temp
                .path()
                .join(format!(".manifest.json.tmp-{}", std::process::id()))
                .exists()
        );
    }

    #[test]
    fn replaces_existing_json_file() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("manifest.json");

        write_json(&path, &Manifest::default()).expect("first write");
        let updated = Manifest {
            schema_version: 2,
            ..Manifest::default()
        };
        write_json(&path, &updated).expect("replace write");

        let parsed: Manifest = serde_json::from_slice(&fs::read(&path).expect("read manifest"))
            .expect("parse manifest");
        assert_eq!(parsed.schema_version, 2);
    }

    #[test]
    fn write_json_new_refuses_existing_destination() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("manifest.json");

        write_json_new(&path, &Manifest::default()).expect("first write");
        let err = write_json_new(&path, &Manifest::default()).expect_err("second write");
        assert!(
            matches!(
                err,
                DotallError::Io {
                    ref source,
                    ..
                } if source.kind() == std::io::ErrorKind::AlreadyExists
            ),
            "expected AlreadyExists, got {err:?}"
        );
    }
}
