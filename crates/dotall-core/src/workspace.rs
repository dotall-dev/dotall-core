use std::path::{Path, PathBuf};

use crate::ALL_DIR_NAME;
use crate::error::{DotallError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn at(root: impl AsRef<Path>) -> Result<Self> {
        let root = root
            .as_ref()
            .canonicalize()
            .map_err(|source| DotallError::io(root.as_ref(), source))?;
        Ok(Self { root })
    }

    pub fn discover(start: impl AsRef<Path>) -> Result<Self> {
        let start = start
            .as_ref()
            .canonicalize()
            .map_err(|source| DotallError::io(start.as_ref(), source))?;
        let start = if start.is_file() {
            start
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| DotallError::InvalidWorkspacePath(start.clone()))?
        } else {
            start
        };

        for candidate in start.ancestors() {
            if candidate.join(ALL_DIR_NAME).join("manifest.json").is_file() {
                return Ok(Self {
                    root: candidate.to_path_buf(),
                });
            }
        }

        Err(DotallError::WorkspaceNotInitialized(start))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn all_dir(&self) -> PathBuf {
        self.root.join(ALL_DIR_NAME)
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.all_dir().join("manifest.json")
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.all_dir().join("objects")
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::Workspace;

    #[test]
    fn discovers_workspace_from_nested_directory() {
        let temp = tempdir().expect("tempdir");
        let nested = temp.path().join("a/b");
        fs::create_dir_all(&nested).expect("nested directory");
        fs::create_dir_all(temp.path().join(".all")).expect(".all");
        fs::write(temp.path().join(".all/manifest.json"), "{}").expect("manifest");

        let workspace = Workspace::discover(&nested).expect("workspace");

        assert_eq!(
            workspace.root(),
            temp.path().canonicalize().expect("canonical root")
        );
    }

    #[test]
    fn rejects_directory_outside_a_workspace() {
        let temp = tempdir().expect("tempdir");

        let error = Workspace::discover(temp.path()).expect_err("not initialized");

        assert!(error.to_string().contains("not initialized"));
    }
}
