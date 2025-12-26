use super::super::{CallToolResult, Content};
use serde_json::json;

pub(super) fn tool_error(code: &'static str, message: impl Into<String>) -> CallToolResult {
    let message = message.into();
    let mut result = CallToolResult::error(vec![Content::text(message.clone())]);
    result.structured_content = Some(json!({
        "error": {
            "code": code,
            "message": message,
        }
    }));
    result
}

pub(super) fn invalid_request(message: impl Into<String>) -> CallToolResult {
    tool_error("invalid_request", message)
}

pub(super) fn invalid_cursor(message: impl Into<String>) -> CallToolResult {
    tool_error("invalid_cursor", message)
}

pub(super) fn internal_error(message: impl Into<String>) -> CallToolResult {
    tool_error("internal", message)
}
