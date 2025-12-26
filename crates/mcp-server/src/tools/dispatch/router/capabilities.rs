use super::super::{CallToolResult, Content, ContextFinderService};
use crate::tools::schemas::capabilities::{CapabilitiesRequest, CapabilitiesResult};
use context_indexer::INDEX_STATE_SCHEMA_VERSION;
use context_protocol::{
    Capabilities, CapabilitiesServer, CapabilitiesVersions, DefaultBudgets, ToolNextAction,
    CAPABILITIES_SCHEMA_VERSION,
};
use serde_json::json;

use super::error::{invalid_request_with_meta, meta_for_request};

/// Return tool capabilities and default budgets for self-directed clients.
pub(in crate::tools::dispatch) async fn capabilities(
    service: &ContextFinderService,
    request: CapabilitiesRequest,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let (root, root_display) = match service.resolve_root(request.path.as_deref()).await {
        Ok(value) => value,
        Err(message) => {
            let meta = meta_for_request(service, request.path.as_deref()).await;
            return Ok(invalid_request_with_meta(message, meta, None, Vec::new()));
        }
    };

    let budgets = DefaultBudgets::default();
    let start_route = ToolNextAction {
        tool: "repo_onboarding_pack".to_string(),
        args: json!({
            "path": root_display,
            "max_chars": budgets.repo_onboarding_pack_max_chars
        }),
        reason: "Start with a compact repo map + key docs (onboarding pack).".to_string(),
    };

    let output = Capabilities {
        schema_version: CAPABILITIES_SCHEMA_VERSION,
        server: CapabilitiesServer {
            name: "context-finder-mcp".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        versions: CapabilitiesVersions {
            command_api: "v1".to_string(),
            mcp: "v2".to_string(),
            index_state: INDEX_STATE_SCHEMA_VERSION,
        },
        default_budgets: budgets,
        start_route,
    };

    let result = CapabilitiesResult {
        capabilities: output,
        meta: service.tool_meta(&root).await,
    };

    Ok(CallToolResult::success(vec![Content::text(
        context_protocol::serialize_json(&result).unwrap_or_default(),
    )]))
}
