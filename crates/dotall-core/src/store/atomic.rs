use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

use serde::Serialize;

use crate::error::{DotallError, Result};

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().ok_or_else(|| DotallError::InvalidWorkspacePath(
        path.to_path_buf(),
    ))?;
    fs::create_dir_all(parent).map_err(|source| DotallError::io(parent, source))?;

    let file_name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
        DotallError::InvalidWorkspacePath(path.to_path_buf())
    })?;
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

        fs::rename(&temporary, path).map_err(|source| DotallError::io(path, source))?;
        sync_parent(parent)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
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

    use crate::manifest::Manifest;

    use super::write_json;

    #[test]
    fn writes_complete_json_and_removes_temporary_file() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("manifest.json");

        write_json(&path, &Manifest::default()).expect("write manifest");

        let parsed: Manifest = serde_json::from_slice(
            &fs::read(&path).expect("read manifest"),
        )
        .expect("parse manifest");
        assert_eq!(parsed, Manifest::default());
        assert!(!temp.path().join(format!(
            ".manifest.json.tmp-{}",
            std::process::id()
        )).exists());
    }
}
