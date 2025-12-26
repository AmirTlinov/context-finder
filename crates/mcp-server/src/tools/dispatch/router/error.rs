use super::super::{CallToolResult, Content, ContextFinderService};
use context_indexer::ToolMeta;
use context_protocol::{DefaultBudgets, ErrorEnvelope, ToolNextAction};
use serde_json::json;

pub(super) fn tool_error_envelope(error: ErrorEnvelope) -> CallToolResult {
    tool_error_envelope_with_meta(error, ToolMeta { index_state: None })
}

pub(super) fn tool_error_envelope_with_meta(
    error: ErrorEnvelope,
    meta: ToolMeta,
) -> CallToolResult {
    let mut result = CallToolResult::error(vec![Content::text(error.message.clone())]);
    result.structured_content = Some(json!({ "error": error, "meta": meta }));
    result
}

pub(super) fn tool_error(code: &'static str, message: impl Into<String>) -> CallToolResult {
    tool_error_envelope(ErrorEnvelope {
        code: code.to_string(),
        message: message.into(),
        details: None,
        hint: None,
        next_actions: Vec::new(),
    })
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

pub(super) fn invalid_cursor_with_meta(
    message: impl Into<String>,
    meta: ToolMeta,
) -> CallToolResult {
    tool_error_envelope_with_meta(
        ErrorEnvelope {
            code: "invalid_cursor".to_string(),
            message: message.into(),
            details: None,
            hint: None,
            next_actions: Vec::new(),
        },
        meta,
    )
}

pub(super) fn invalid_request_with_meta(
    message: impl Into<String>,
    meta: ToolMeta,
    hint: Option<String>,
    next_actions: Vec<ToolNextAction>,
) -> CallToolResult {
    tool_error_envelope_with_meta(
        ErrorEnvelope {
            code: "invalid_request".to_string(),
            message: message.into(),
            details: None,
            hint,
            next_actions,
        },
        meta,
    )
}

pub(super) fn internal_error_with_meta(
    message: impl Into<String>,
    meta: ToolMeta,
) -> CallToolResult {
    tool_error_envelope_with_meta(
        ErrorEnvelope {
            code: "internal".to_string(),
            message: message.into(),
            details: None,
            hint: None,
            next_actions: Vec::new(),
        },
        meta,
    )
}

pub(super) fn invalid_request_with(
    message: impl Into<String>,
    hint: Option<String>,
    next_actions: Vec<ToolNextAction>,
) -> CallToolResult {
    tool_error_envelope(ErrorEnvelope {
        code: "invalid_request".to_string(),
        message: message.into(),
        details: None,
        hint,
        next_actions,
    })
}

pub(super) fn index_recovery_actions(root_display: &str) -> Vec<ToolNextAction> {
    let budgets = DefaultBudgets::default();
    vec![
        ToolNextAction {
            tool: "index".to_string(),
            args: json!({ "path": root_display }),
            reason: format!(
                "Build the semantic index (recommended auto_index_budget_ms={}).",
                budgets.auto_index_budget_ms
            ),
        },
        ToolNextAction {
            tool: "doctor".to_string(),
            args: json!({ "path": root_display }),
            reason: "Check environment + index state before retrying.".to_string(),
        },
    ]
}

pub(super) fn attach_meta(mut result: CallToolResult, meta: ToolMeta) -> CallToolResult {
    let value = result.structured_content.get_or_insert_with(|| json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert("meta".to_string(), json!(meta));
    }
    result
}

pub(super) async fn meta_for_request(
    service: &ContextFinderService,
    path: Option<&str>,
) -> ToolMeta {
    match resolve_root_for_meta(service, path).await {
        Some(root) => service.tool_meta(&root).await,
        None => ToolMeta { index_state: None },
    }
}

async fn resolve_root_for_meta(
    service: &ContextFinderService,
    path: Option<&str>,
) -> Option<std::path::PathBuf> {
    service.resolve_root(path).await.ok().map(|(root, _)| root)
}
