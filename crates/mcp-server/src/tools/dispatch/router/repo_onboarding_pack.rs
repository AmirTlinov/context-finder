use super::super::{
    compute_repo_onboarding_pack_result, CallToolResult, Content, ContextFinderService, McpError,
    RepoOnboardingPackRequest,
};
use std::path::PathBuf;

/// Repo onboarding pack (map + key docs slices + next actions).
pub(in crate::tools::dispatch) async fn repo_onboarding_pack(
    service: &ContextFinderService,
    request: RepoOnboardingPackRequest,
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
    let mut result = match compute_repo_onboarding_pack_result(&root, &root_display, &request).await
    {
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
