use super::super::{
    compute_file_slice_result, CallToolResult, Content, ContextFinderService, FileSliceRequest,
    McpError,
};
use crate::tools::schemas::ToolNextAction;
use serde_json::json;

use super::error::{invalid_request_with_meta, meta_for_request};

/// Read a bounded slice of a file within the project root (safe file access for agents).
pub(in crate::tools::dispatch) async fn file_slice(
    service: &ContextFinderService,
    request: &FileSliceRequest,
) -> Result<CallToolResult, McpError> {
    let (root, root_display) = match service.resolve_root(request.path.as_deref()).await {
        Ok(value) => value,
        Err(message) => {
            let meta = meta_for_request(service, request.path.as_deref()).await;
            return Ok(invalid_request_with_meta(message, meta, None, Vec::new()));
        }
    };
    let meta = service.tool_meta(&root).await;
    let mut result = match compute_file_slice_result(&root, &root_display, request) {
        Ok(result) => result,
        Err(msg) => {
            return Ok(invalid_request_with_meta(
                msg,
                meta.clone(),
                None,
                Vec::new(),
            ));
        }
    };
    result.meta = meta;
    if let Some(cursor) = result.next_cursor.clone() {
        result.next_actions = Some(vec![ToolNextAction {
            tool: "file_slice".to_string(),
            args: json!({
                "path": root_display,
                "file": result.file.clone(),
                "max_lines": result.max_lines,
                "max_chars": result.max_chars,
                "cursor": cursor,
            }),
            reason: "Continue file_slice pagination with the next cursor.".to_string(),
        }]);
    }

    Ok(CallToolResult::success(vec![Content::text(
        context_protocol::serialize_json(&result).unwrap_or_default(),
    )]))
}
