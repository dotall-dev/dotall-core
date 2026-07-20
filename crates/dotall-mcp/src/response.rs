use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
#[schemars(extend("type" = "object"))]
pub enum ToolResponse<T> {
    Success {
        result: T,
        next_actions: Vec<String>,
    },
    Error {
        code: String,
        message: String,
        retryable: bool,
        next_actions: Vec<String>,
        details: serde_json::Value,
    },
}

impl<T> ToolResponse<T> {
    pub fn success(result: T, next_actions: Vec<String>) -> Self {
        Self::Success {
            result,
            next_actions,
        }
    }

    pub fn error(
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        next_actions: Vec<String>,
        details: serde_json::Value,
    ) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
            retryable,
            next_actions,
            details,
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct JsonResult {
    pub data: serde_json::Value,
}
