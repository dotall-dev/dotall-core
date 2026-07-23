use std::path::Path;

use dotall_core::{DotallError, Result};

use crate::response::ToolResponse;

/// Resolves an absolute or workspace-relative file path to a UTF-8 workspace key.
pub fn relative_path_for_file(workspace_root: &Path, file: impl AsRef<Path>) -> Result<String> {
    let file = file.as_ref();
    let canonical = file.canonicalize().map_err(|source| DotallError::Io {
        path: file.to_path_buf(),
        source,
    })?;
    let relative =
        canonical
            .strip_prefix(workspace_root)
            .map_err(|_| DotallError::InvalidSourcePath {
                path: canonical.clone(),
                reason: "file is outside the Dotall workspace".into(),
            })?;
    relative_path_from_stripped(relative, &canonical)
}

fn relative_path_from_stripped(relative: &Path, canonical: &Path) -> Result<String> {
    let key = relative
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            std::path::Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");

    if key.is_empty() {
        return Err(DotallError::InvalidSourcePath {
            path: canonical.to_path_buf(),
            reason: "path must refer to a file inside the workspace".into(),
        });
    }

    Ok(key)
}

/// Runs blocking engine and filesystem work away from the async MCP runtime.
pub async fn blocking<T, F>(operation: F) -> ToolResponse<T>
where
    T: Send + 'static,
    F: FnOnce() -> dotall_core::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(operation).await {
        Ok(Ok(value)) => ToolResponse::success(value, Vec::new()),
        Ok(Err(error)) => crate::error::tool_error_typed(error),
        Err(error) => ToolResponse::error(
            "worker_failed",
            error.to_string(),
            true,
            vec!["Retry the same idempotent request.".into()],
            serde_json::json!({}),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn relative_path_for_file_strips_workspace_root() {
        let temp = tempdir().expect("tempdir");
        let nested = temp.path().join("docs");
        fs::create_dir_all(&nested).expect("nested directory");
        let file = nested.join("book.xlsx");
        fs::write(&file, b"data").expect("source file");

        let root = temp.path().canonicalize().expect("workspace root");
        let relative = relative_path_for_file(&root, &file).expect("relative path");

        assert_eq!(relative, "docs/book.xlsx");
    }
}
