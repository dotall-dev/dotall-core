//! Local writer lock for apply — one concurrent apply per tracked object.
//!
//! Journals live under `state/transactions/`; this lock guards apply against
//! concurrent writers on the same tracked object.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::error::{DotallError, Result};

/// RAII exclusive advisory lock for apply on one tracked object.
///
/// Lock file path: `<object>/state/edits/.apply.lock`.
pub struct ApplyLock {
    file: File,
    path: PathBuf,
}

impl ApplyLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| DotallError::io(parent, source))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| DotallError::io(path, source))?;
        file.try_lock_exclusive()
            .map_err(|_| DotallError::LockBusy {
                path: path.to_path_buf(),
            })?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }
}

impl Drop for ApplyLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let _ = std::fs::remove_file(&self.path);
    }
}
