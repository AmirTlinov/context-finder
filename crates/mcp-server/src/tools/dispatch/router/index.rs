use super::super::{
    current_model_id, index_path_for_model, CallToolResult, Content, ContextFinderService,
    IndexRequest, IndexResult, McpError, QueryKind,
};
use context_protocol::{DefaultBudgets, ToolNextAction};
use serde_json::json;
use std::collections::HashSet;

use super::error::{internal_error_with_meta, invalid_request_with_meta, meta_for_request};
/// Index a project
pub(in crate::tools::dispatch) async fn index(
    service: &ContextFinderService,
    request: IndexRequest,
) -> Result<CallToolResult, McpError> {
    let force = request.force.unwrap_or(false);
    let full = request.full.unwrap_or(false) || force;
    let experts = request.experts.unwrap_or(false);
    let extra_models = request.models.unwrap_or_default();

    let (canonical, root_display) = match service.resolve_root(request.path.as_deref()).await {
        Ok(value) => value,
        Err(message) => {
            let meta = meta_for_request(service, request.path.as_deref()).await;
            return Ok(invalid_request_with_meta(message, meta, None, Vec::new()));
        }
    };
    let meta = service.tool_meta(&canonical).await;

    let start = std::time::Instant::now();

    let primary_model_id = current_model_id().unwrap_or_else(|_| "bge-small".to_string());
    let templates = service.profile.embedding().clone();

    let mut models: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    seen.insert(primary_model_id.clone());
    models.push(primary_model_id.clone());

    if experts {
        let expert_cfg = service.profile.experts();
        for kind in [
            QueryKind::Identifier,
            QueryKind::Path,
            QueryKind::Conceptual,
        ] {
            for model_id in expert_cfg.semantic_models(kind) {
                if seen.insert(model_id.clone()) {
                    models.push(model_id.clone());
                }
            }
        }
    }

    for model_id in extra_models {
        if seen.insert(model_id.clone()) {
            models.push(model_id);
        }
    }

    let registry = match context_vector_store::ModelRegistry::from_env() {
        Ok(r) => r,
        Err(e) => {
            return Ok(internal_error_with_meta(
                format!("Model registry error: {e}"),
                meta.clone(),
            ));
        }
    };
    for model_id in &models {
        if let Err(e) = registry.dimension(model_id) {
            return Ok(invalid_request_with_meta(
                format!("Unknown or unsupported model_id '{model_id}': {e}"),
                meta.clone(),
                None,
                Vec::new(),
            ));
        }
    }

    let specs: Vec<context_indexer::ModelIndexSpec> = models
        .iter()
        .map(|model_id| context_indexer::ModelIndexSpec::new(model_id.clone(), templates.clone()))
        .collect();

    let indexer = match context_indexer::MultiModelProjectIndexer::new(&canonical).await {
        Ok(i) => i,
        Err(e) => {
            return Ok(internal_error_with_meta(
                format!("Indexer init error: {e}"),
                meta.clone(),
            ));
        }
    };

    let stats = match indexer.index_models(&specs, full).await {
        Ok(s) => s,
        Err(e) => {
            return Ok(internal_error_with_meta(
                format!("Indexing error: {e}"),
                meta.clone(),
            ));
        }
    };

    let time_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let index_path = index_path_for_model(&canonical, &primary_model_id);

    let mut result = IndexResult {
        files: stats.files,
        chunks: stats.chunks,
        time_ms,
        index_path: index_path.to_string_lossy().to_string(),
        next_actions: Vec::new(),
        meta: service.tool_meta(&canonical).await,
    };
    let budgets = DefaultBudgets::default();
    result.next_actions.push(ToolNextAction {
        tool: "repo_onboarding_pack".to_string(),
        args: json!({
            "path": root_display.clone(),
            "max_chars": budgets.repo_onboarding_pack_max_chars
        }),
        reason: "Start with a compact repo map + key docs after indexing.".to_string(),
    });
    result.next_actions.push(ToolNextAction {
        tool: "context_pack".to_string(),
        args: json!({
            "path": root_display.clone(),
            "query": "project overview",
            "max_chars": budgets.context_pack_max_chars
        }),
        reason: "Build a bounded semantic overview after indexing.".to_string(),
    });

    Ok(CallToolResult::success(vec![Content::text(
        context_protocol::serialize_json(&result).unwrap_or_default(),
    )]))
}
