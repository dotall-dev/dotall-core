use dotall_core::DotallError;

use crate::response::{JsonResult, ToolResponse};

pub fn tool_error(error: DotallError) -> ToolResponse<JsonResult> {
    tool_error_typed(error)
}

pub fn tool_error_typed<T>(error: DotallError) -> ToolResponse<T> {
    match error {
        DotallError::SourceHashMismatch {
            path,
            expected,
            actual,
        } => ToolResponse::Error {
            code: "stale_source".into(),
            message: format!("{} changed since the agent inspected it", path.display()),
            retryable: true,
            next_actions: vec![
                "Inspect the file again.".into(),
                "Rebuild the edit with the new source hash.".into(),
            ],
            details: serde_json::json!({
                "path": path.display().to_string(),
                "expected_hash": expected,
                "actual_hash": actual,
            }),
        },
        DotallError::ApplyAllPartial {
            path,
            applied,
            failed_tx,
            source,
        } => ToolResponse::Error {
            code: "apply_all_partial".into(),
            message: format!(
                "apply-all stopped at transaction {failed_tx} after committing {} edit(s): {source}",
                applied.len()
            ),
            retryable: true,
            next_actions: vec![
                "Inspect the current file state and the remaining staged edit.".into(),
                "Discard or replace the failed transaction, then retry apply-all.".into(),
            ],
            details: serde_json::json!({
                "path": path.display().to_string(),
                "applied": applied,
                "failed_transaction_id": failed_tx,
                "cause": source.to_string(),
            }),
        },
        DotallError::UnsupportedCapability {
            format_id,
            capability,
            available,
        } => ToolResponse::Error {
            code: "unsupported_capability".into(),
            message: format!("{format_id} does not support {capability}"),
            retryable: false,
            next_actions: available,
            details: serde_json::json!({
                "format_id": format_id,
                "capability": capability,
            }),
        },
        DotallError::StagedConflict { path, tx_id } => ToolResponse::Error {
            code: "staged_conflict".into(),
            message: format!(
                "staged edit {tx_id} for {} conflicts with an existing staged payload",
                path.display()
            ),
            retryable: true,
            next_actions: vec![
                "Reuse the same transaction_id with an identical payload.".into(),
                "Discard the conflicting staged edit before retrying.".into(),
            ],
            details: serde_json::json!({
                "path": path.display().to_string(),
                "transaction_id": tx_id,
            }),
        },
        DotallError::StagedMissing { path, tx_id } => ToolResponse::Error {
            code: "staged_missing".into(),
            message: format!("staged edit {tx_id} is missing for {}", path.display()),
            retryable: false,
            next_actions: vec!["List staged edits and retry with a valid transaction_id.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
                "transaction_id": tx_id,
            }),
        },
        DotallError::LockBusy { path } => ToolResponse::Error {
            code: "lock_busy".into(),
            message: format!("apply lock is held for {}", path.display()),
            retryable: true,
            next_actions: vec!["Retry apply after the in-flight apply completes.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
            }),
        },
        DotallError::WorkspaceNotInitialized(path) => ToolResponse::Error {
            code: "workspace_not_initialized".into(),
            message: format!("Dotall is not initialized at or above {}", path.display()),
            retryable: false,
            next_actions: vec!["Call init on the workspace root.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
            }),
        },
        DotallError::InvalidSourcePath { path, reason } => ToolResponse::Error {
            code: "invalid_source_path".into(),
            message: format!("invalid source path {}: {reason}", path.display()),
            retryable: false,
            next_actions: vec!["Use a file path inside an initialized Dotall workspace.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
                "reason": reason,
            }),
        },
        DotallError::UnsupportedFormat(path) => ToolResponse::Error {
            code: "unsupported_format".into(),
            message: format!("unsupported file format: {}", path.display()),
            retryable: false,
            next_actions: vec!["Inspect capabilities for supported formats.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
            }),
        },
        DotallError::SourceChangedDuringRead(path) => ToolResponse::Error {
            code: "source_changed_during_read".into(),
            message: format!(
                "source changed while it was being hashed: {}",
                path.display()
            ),
            retryable: true,
            next_actions: vec!["Inspect the file again and retry.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
            }),
        },
        DotallError::Io { path, source } => ToolResponse::Error {
            code: "io_error".into(),
            message: format!("I/O operation failed at {}: {source}", path.display()),
            retryable: true,
            next_actions: vec!["Verify the file exists and is readable, then retry.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
            }),
        },
        DotallError::Format {
            format_id,
            path,
            message,
        } => ToolResponse::Error {
            code: "format_error".into(),
            message: format!(
                "{format_id} processing failed for {}: {message}",
                path.display()
            ),
            retryable: false,
            next_actions: vec!["Inspect the file and validate edit payloads.".into()],
            details: serde_json::json!({
                "format_id": format_id,
                "path": path.display().to_string(),
            }),
        },
        DotallError::Serialization { context, source } => ToolResponse::Error {
            code: "serialization_error".into(),
            message: format!("failed to serialize {context}: {source}"),
            retryable: false,
            next_actions: vec!["Verify request payloads are valid JSON.".into()],
            details: serde_json::json!({
                "context": context,
            }),
        },
        DotallError::SnapshotMissing { path, hash } => ToolResponse::Error {
            code: "snapshot_missing".into(),
            message: format!("snapshot {hash} is missing for {}", path.display()),
            retryable: false,
            next_actions: vec!["Inspect history and choose a valid version.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
                "hash": hash,
            }),
        },
        DotallError::HistoryVersionMissing { path, version } => ToolResponse::Error {
            code: "history_version_missing".into(),
            message: format!(
                "history version {version} is missing for {}",
                path.display()
            ),
            retryable: false,
            next_actions: vec!["List history and choose a valid version.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
                "version": version,
            }),
        },
        DotallError::HistoryVersionExists { path, version } => ToolResponse::Error {
            code: "history_version_exists".into(),
            message: format!(
                "history version {version} already exists for {}",
                path.display()
            ),
            retryable: false,
            next_actions: vec!["Inspect history for the current version state.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
                "version": version,
            }),
        },
        DotallError::InvalidManifest { path, source } => ToolResponse::Error {
            code: "invalid_manifest".into(),
            message: format!("manifest at {} is invalid: {source}", path.display()),
            retryable: false,
            next_actions: vec!["Reinitialize the workspace or repair .all/manifest.json.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
            }),
        },
        DotallError::UnsupportedManifestSchema { found, supported } => ToolResponse::Error {
            code: "unsupported_manifest_schema".into(),
            message: format!(
                "unsupported manifest schema {found}; this build supports {supported}"
            ),
            retryable: false,
            next_actions: vec![
                "Upgrade dotall-mcp or reinitialize with a supported schema.".into(),
            ],
            details: serde_json::json!({
                "found": found,
                "supported": supported,
            }),
        },
        DotallError::InvalidWorkspacePath(path) => ToolResponse::Error {
            code: "invalid_workspace_path".into(),
            message: format!("invalid workspace path: {}", path.display()),
            retryable: false,
            next_actions: vec!["Provide a valid workspace directory path.".into()],
            details: serde_json::json!({
                "path": path.display().to_string(),
            }),
        },
        DotallError::ArtifactSchemaMismatch {
            format_id,
            schema_id,
            schema_version,
        } => ToolResponse::Error {
            code: "artifact_schema_mismatch".into(),
            message: format!(
                "format {format_id} rejected artifact schema {schema_id} v{schema_version}"
            ),
            retryable: false,
            next_actions: vec!["Re-inspect the file to refresh cached artifacts.".into()],
            details: serde_json::json!({
                "format_id": format_id,
                "schema_id": schema_id,
                "schema_version": schema_version,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn maps_stale_source_to_retryable_error() {
        let response = tool_error(DotallError::SourceHashMismatch {
            path: PathBuf::from("book.xlsx"),
            expected: "abc".into(),
            actual: "def".into(),
        });
        match response {
            ToolResponse::Error {
                code,
                retryable,
                details,
                ..
            } => {
                assert_eq!(code, "stale_source");
                assert!(retryable);
                assert_eq!(details["expected_hash"], "abc");
                assert_eq!(details["actual_hash"], "def");
            }
            ToolResponse::Success { .. } => panic!("expected error response"),
        }
    }
}
