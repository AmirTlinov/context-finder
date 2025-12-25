use super::super::{
    compute_map_result, decode_map_cursor, CallToolResult, Content, ContextFinderService,
    MapRequest, McpError, CURSOR_VERSION,
};
use std::path::PathBuf;

/// Get project structure overview
pub(in crate::tools::dispatch) async fn map(
    service: &ContextFinderService,
    request: MapRequest,
) -> Result<CallToolResult, McpError> {
    let path = PathBuf::from(request.path.unwrap_or_else(|| ".".to_string()));
    let depth = request.depth.unwrap_or(2).clamp(1, 4);
    let limit = request.limit.unwrap_or(10);

    let root = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Invalid path: {e}"
            ))]));
        }
    };
    ContextFinderService::touch_daemon_best_effort(&root);
    let root_display = root.to_string_lossy().to_string();

    let offset = if let Some(cursor) = request
        .cursor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let decoded = match decode_map_cursor(cursor) {
            Ok(v) => v,
            Err(err) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Invalid cursor: {err}"
                ))]));
            }
        };
        if decoded.v != CURSOR_VERSION || decoded.tool != "map" {
            return Ok(CallToolResult::error(vec![Content::text(
                "Invalid cursor: wrong tool",
            )]));
        }
        if decoded.root != root_display {
            return Ok(CallToolResult::error(vec![Content::text(
                "Invalid cursor: different root",
            )]));
        }
        if decoded.depth != depth {
            return Ok(CallToolResult::error(vec![Content::text(
                "Invalid cursor: different depth",
            )]));
        }
        decoded.offset
    } else {
        0usize
    };

    let mut result = match compute_map_result(&root, &root_display, depth, limit, offset).await {
        Ok(result) => result,
        Err(err) => {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {err:#}"
            ))]));
        }
    };
    result.meta = Some(service.tool_meta(&root).await);

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
    )]))
}
