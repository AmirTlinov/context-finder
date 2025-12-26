use super::super::{
    compute_repo_onboarding_pack_result, AutoIndexPolicy, CallToolResult, Content,
    ContextFinderService, McpError, RepoOnboardingPackRequest,
};

/// Repo onboarding pack (map + key docs slices + next actions).
pub(in crate::tools::dispatch) async fn repo_onboarding_pack(
    service: &ContextFinderService,
    request: RepoOnboardingPackRequest,
) -> Result<CallToolResult, McpError> {
    let (root, root_display) = match service.resolve_root(request.path.as_deref()).await {
        Ok(value) => value,
        Err(message) => return Ok(CallToolResult::error(vec![Content::text(message)])),
    };
    let policy = AutoIndexPolicy::from_request(request.auto_index, request.auto_index_budget_ms);
    let meta = service.tool_meta_with_auto_index(&root, policy).await;
    let mut result = match compute_repo_onboarding_pack_result(&root, &root_display, &request).await
    {
        Ok(result) => result,
        Err(err) => {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {err:#}"
            ))]));
        }
    };
    result.meta = Some(meta);

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
    )]))
}
