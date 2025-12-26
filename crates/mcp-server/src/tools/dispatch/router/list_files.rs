use super::super::{
    compute_list_files_result, decode_list_files_cursor, finalize_list_files_budget,
    CallToolResult, Content, ContextFinderService, ListFilesRequest, McpError, CURSOR_VERSION,
};
use crate::tools::schemas::ToolNextAction;
use serde_json::json;

use super::error::{
    internal_error_with_meta, invalid_cursor_with_meta, invalid_request_with_meta, meta_for_request,
};

/// List project files within the project root (safe file enumeration for agents).
pub(in crate::tools::dispatch) async fn list_files(
    service: &ContextFinderService,
    request: ListFilesRequest,
) -> Result<CallToolResult, McpError> {
    const DEFAULT_LIMIT: usize = 200;
    const MAX_LIMIT: usize = 50_000;
    const DEFAULT_MAX_CHARS: usize = 20_000;
    const MAX_MAX_CHARS: usize = 500_000;

    let (root, root_display) = match service.resolve_root(request.path.as_deref()).await {
        Ok(value) => value,
        Err(message) => {
            let meta = meta_for_request(service, request.path.as_deref()).await;
            return Ok(invalid_request_with_meta(message, meta, None, Vec::new()));
        }
    };
    let meta = service.tool_meta(&root).await;

    let limit = request.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let max_chars = request
        .max_chars
        .unwrap_or(DEFAULT_MAX_CHARS)
        .clamp(1, MAX_MAX_CHARS);

    let normalized_file_pattern = request
        .file_pattern
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let cursor_last_file = if let Some(cursor) = request
        .cursor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let decoded = match decode_list_files_cursor(cursor) {
            Ok(v) => v,
            Err(err) => {
                return Ok(invalid_cursor_with_meta(
                    format!("Invalid cursor: {err}"),
                    meta.clone(),
                ));
            }
        };
        if decoded.v != CURSOR_VERSION || decoded.tool != "list_files" {
            return Ok(invalid_cursor_with_meta(
                "Invalid cursor: wrong tool",
                meta.clone(),
            ));
        }
        if decoded.root != root_display {
            return Ok(invalid_cursor_with_meta(
                "Invalid cursor: different root",
                meta.clone(),
            ));
        }
        if decoded.file_pattern != normalized_file_pattern {
            return Ok(invalid_cursor_with_meta(
                "Invalid cursor: different file_pattern",
                meta.clone(),
            ));
        }
        Some(decoded.last_file)
    } else {
        None
    };
    let mut result = match compute_list_files_result(
        &root,
        &root_display,
        request.file_pattern.as_deref(),
        limit,
        max_chars,
        cursor_last_file.as_deref(),
    )
    .await
    {
        Ok(result) => result,
        Err(err) => {
            return Ok(internal_error_with_meta(
                format!("Error: {err:#}"),
                meta.clone(),
            ))
        }
    };
    result.meta = meta.clone();
    if let Some(cursor) = result.next_cursor.clone() {
        result.next_actions = Some(vec![ToolNextAction {
            tool: "list_files".to_string(),
            args: json!({
                "path": root_display,
                "file_pattern": normalized_file_pattern,
                "limit": limit,
                "max_chars": max_chars,
                "cursor": cursor,
            }),
            reason: "Continue list_files pagination with the next cursor.".to_string(),
        }]);
    }
    if let Err(err) = finalize_list_files_budget(&mut result) {
        let suggested = max_chars.saturating_mul(2).clamp(1, MAX_MAX_CHARS);
        return Ok(invalid_request_with_meta(
            format!("max_chars too small for response envelope ({err:#})"),
            meta,
            Some(format!("Increase max_chars (suggested: {suggested}).")),
            vec![ToolNextAction {
                tool: "list_files".to_string(),
                args: json!({
                    "path": root_display,
                    "file_pattern": request.file_pattern,
                    "limit": limit,
                    "max_chars": suggested,
                    "cursor": request.cursor
                }),
                reason: "Retry list_files with a larger max_chars budget.".to_string(),
            }],
        ));
    }

    Ok(CallToolResult::success(vec![Content::text(
        context_protocol::serialize_json(&result).unwrap_or_default(),
    )]))
}
