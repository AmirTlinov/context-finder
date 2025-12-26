use super::super::{
    compute_map_result, decode_map_cursor, CallToolResult, Content, ContextFinderService,
    MapRequest, McpError, CURSOR_VERSION,
};
use crate::tools::schemas::ToolNextAction;
use serde_json::json;

use super::error::{internal_error, invalid_cursor, invalid_request};

/// Get project structure overview
pub(in crate::tools::dispatch) async fn map(
    service: &ContextFinderService,
    request: MapRequest,
) -> Result<CallToolResult, McpError> {
    let depth = request.depth.unwrap_or(2).clamp(1, 4);
    let limit = request.limit.unwrap_or(10);

    let (root, root_display) = match service.resolve_root(request.path.as_deref()).await {
        Ok(value) => value,
        Err(message) => return Ok(invalid_request(message)),
    };

    let offset = if let Some(cursor) = request
        .cursor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let decoded = match decode_map_cursor(cursor) {
            Ok(v) => v,
            Err(err) => {
                return Ok(invalid_cursor(format!("Invalid cursor: {err}")));
            }
        };
        if decoded.v != CURSOR_VERSION || decoded.tool != "map" {
            return Ok(invalid_cursor("Invalid cursor: wrong tool"));
        }
        if decoded.root != root_display {
            return Ok(invalid_cursor("Invalid cursor: different root"));
        }
        if decoded.depth != depth {
            return Ok(invalid_cursor("Invalid cursor: different depth"));
        }
        decoded.offset
    } else {
        0usize
    };

    let mut result = match compute_map_result(&root, &root_display, depth, limit, offset).await {
        Ok(result) => result,
        Err(err) => {
            return Ok(internal_error(format!("Error: {err:#}")));
        }
    };
    result.meta = Some(service.tool_meta(&root).await);
    if let Some(cursor) = result.next_cursor.clone() {
        result.next_actions = Some(vec![ToolNextAction {
            tool: "map".to_string(),
            args: json!({
                "path": root_display,
                "depth": depth,
                "limit": limit,
                "cursor": cursor,
            }),
            reason: "Continue map pagination with the next cursor.".to_string(),
        }]);
    }

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
    )]))
}
