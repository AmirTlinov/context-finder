use super::super::{
    compute_repo_onboarding_pack_result, AutoIndexPolicy, CallToolResult, Content,
    ContextFinderService, McpError, RepoOnboardingPackRequest,
};

use super::error::{internal_error_with_meta, invalid_request_with_meta, meta_for_request};
/// Repo onboarding pack (map + key docs slices + next actions).
pub(in crate::tools::dispatch) async fn repo_onboarding_pack(
    service: &ContextFinderService,
    request: RepoOnboardingPackRequest,
) -> Result<CallToolResult, McpError> {
    let (root, root_display) = match service.resolve_root(request.path.as_deref()).await {
        Ok(value) => value,
        Err(message) => {
            let meta = meta_for_request(service, request.path.as_deref()).await;
            return Ok(invalid_request_with_meta(message, meta, None, Vec::new()));
        }
    };
    let policy = AutoIndexPolicy::from_request(request.auto_index, request.auto_index_budget_ms);
    let meta = service.tool_meta_with_auto_index(&root, policy).await;
    let mut result = match compute_repo_onboarding_pack_result(&root, &root_display, &request).await
    {
        Ok(result) => result,
        Err(err) => {
            return Ok(internal_error_with_meta(
                format!("Error: {err:#}"),
                meta.clone(),
            ));
        }
    };
    result.meta = meta;

    Ok(CallToolResult::success(vec![Content::text(
        context_protocol::serialize_json(&result).unwrap_or_default(),
    )]))
}
