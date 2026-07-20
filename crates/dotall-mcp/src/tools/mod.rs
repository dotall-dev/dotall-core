use crate::response::ToolResponse;

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
