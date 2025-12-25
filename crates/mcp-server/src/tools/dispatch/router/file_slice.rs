use super::super::{
    compute_file_slice_result, CallToolResult, Content, ContextFinderService, FileSliceRequest,
    McpError,
};
use std::path::PathBuf;

/// Read a bounded slice of a file within the project root (safe file access for agents).
pub(in crate::tools::dispatch) async fn file_slice(
    service: &ContextFinderService,
    request: &FileSliceRequest,
) -> Result<CallToolResult, McpError> {
    let root_path = PathBuf::from(request.path.as_deref().unwrap_or("."));
    let root = match root_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Invalid path: {e}"
            ))]));
        }
    };
    ContextFinderService::touch_daemon_best_effort(&root);
    let root_display = root.to_string_lossy().to_string();
    let mut result = match compute_file_slice_result(&root, &root_display, request) {
        Ok(result) => result,
        Err(msg) => return Ok(CallToolResult::error(vec![Content::text(msg)])),
    };
    result.meta = Some(service.tool_meta(&root).await);

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
    )]))
}
