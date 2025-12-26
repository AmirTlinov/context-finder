use super::super::{
    AutoIndexPolicy, CallToolResult, Content, ContextFinderService, McpError, SearchRequest,
    SearchResponse, SearchResult,
};

use super::error::{
    index_recovery_actions, internal_error_with_meta, invalid_request_with_meta, meta_for_request,
    tool_error_envelope_with_meta,
};
use context_protocol::{DefaultBudgets, ErrorEnvelope, ToolNextAction};
/// Semantic code search
pub(in crate::tools::dispatch) async fn search(
    service: &ContextFinderService,
    request: SearchRequest,
) -> Result<CallToolResult, McpError> {
    let limit = request.limit.unwrap_or(10).clamp(1, 50);

    if request.query.trim().is_empty() {
        let meta = meta_for_request(service, request.path.as_deref()).await;
        return Ok(invalid_request_with_meta(
            "Error: Query cannot be empty",
            meta,
            None,
            Vec::new(),
        ));
    }

    let (root, root_display) = match service.resolve_root(request.path.as_deref()).await {
        Ok(value) => value,
        Err(message) => {
            let meta = meta_for_request(service, request.path.as_deref()).await;
            return Ok(invalid_request_with_meta(message, meta, None, Vec::new()));
        }
    };

    let policy = AutoIndexPolicy::from_request(request.auto_index, request.auto_index_budget_ms);
    let (mut engine, meta) = match service.prepare_semantic_engine(&root, policy).await {
        Ok(engine) => engine,
        Err(e) => {
            let message = format!("Error: {e}");
            let meta = service.tool_meta(&root).await;
            if message.contains("Index not found")
                || message.contains("No semantic indices available")
            {
                return Ok(tool_error_envelope_with_meta(
                    ErrorEnvelope {
                        code: "index_missing".to_string(),
                        message,
                        details: None,
                        hint: Some("Index missing — run index (see next_actions).".to_string()),
                        next_actions: index_recovery_actions(&root_display),
                    },
                    meta,
                ));
            }
            return Ok(internal_error_with_meta(message, meta));
        }
    };

    let results = {
        match engine
            .engine_mut()
            .context_search
            .hybrid_mut()
            .search(&request.query, limit)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(internal_error_with_meta(
                    format!("Search error: {e}"),
                    meta.clone(),
                ));
            }
        }
    };

    let formatted: Vec<SearchResult> = results
        .into_iter()
        .map(|r| {
            let chunk = r.chunk;
            SearchResult {
                file: chunk.file_path,
                start_line: chunk.start_line,
                end_line: chunk.end_line,
                symbol: chunk.metadata.symbol_name,
                symbol_type: chunk.metadata.chunk_type.map(|ct| ct.as_str().to_string()),
                score: r.score,
                content: chunk.content,
            }
        })
        .collect();

    let mut next_actions = Vec::new();
    let budgets = DefaultBudgets::default();
    next_actions.push(ToolNextAction {
        tool: "context_pack".to_string(),
        args: serde_json::json!({
            "path": root_display.clone(),
            "query": request.query,
            "max_chars": budgets.context_pack_max_chars
        }),
        reason: "Build a bounded semantic pack for deeper context.".to_string(),
    });
    if let Some(first) = formatted.first() {
        next_actions.push(ToolNextAction {
            tool: "read_pack".to_string(),
            args: serde_json::json!({
                "path": root_display,
                "file": first.file.clone(),
                "start_line": first.start_line,
                "max_chars": budgets.read_pack_max_chars
            }),
            reason: "Open the top hit with a bounded read_pack.".to_string(),
        });
    }

    let response = SearchResponse {
        results: formatted,
        next_actions,
        meta,
    };

    Ok(CallToolResult::success(vec![Content::text(
        context_protocol::serialize_json(&response).unwrap_or_default(),
    )]))
}
