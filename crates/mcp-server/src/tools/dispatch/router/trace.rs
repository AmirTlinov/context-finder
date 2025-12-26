use super::super::{
    AutoIndexPolicy, CallToolResult, Content, ContextFinderService, McpError, TraceRequest,
    TraceResult, TraceStep,
};
use super::error::{internal_error_with_meta, invalid_request_with_meta, meta_for_request};

/// Trace call path between two symbols
pub(in crate::tools::dispatch) async fn trace(
    service: &ContextFinderService,
    request: TraceRequest,
) -> Result<CallToolResult, McpError> {
    let root = match service.resolve_root(request.path.as_deref()).await {
        Ok((root, _)) => root,
        Err(message) => {
            let meta = meta_for_request(service, request.path.as_deref()).await;
            return Ok(invalid_request_with_meta(message, meta, None, Vec::new()));
        }
    };

    let policy = AutoIndexPolicy::from_request(request.auto_index, request.auto_index_budget_ms);
    let (mut engine, meta) = match service.prepare_semantic_engine(&root, policy).await {
        Ok(engine) => engine,
        Err(e) => {
            let meta = service.tool_meta(&root).await;
            return Ok(internal_error_with_meta(format!("Error: {e}"), meta));
        }
    };

    let language = request.language.as_deref().map_or_else(
        || {
            ContextFinderService::detect_language(
                engine.engine_mut().context_search.hybrid().chunks(),
            )
        },
        |lang| ContextFinderService::parse_language(Some(lang)),
    );

    if let Err(e) = engine.engine_mut().ensure_graph(language).await {
        return Ok(internal_error_with_meta(
            format!("Graph build error: {e}"),
            meta.clone(),
        ));
    }

    let (found, path_steps, depth) = {
        let Some(assembler) = engine.engine_mut().context_search.assembler() else {
            return Ok(internal_error_with_meta(
                "Graph build error: missing assembler after build",
                meta.clone(),
            ));
        };
        let graph = assembler.graph();

        // Find both symbols
        let Some(from_node) = graph.find_node(&request.from) else {
            return Ok(invalid_request_with_meta(
                format!("Symbol '{}' not found", request.from),
                meta.clone(),
                None,
                Vec::new(),
            ));
        };

        let Some(to_node) = graph.find_node(&request.to) else {
            return Ok(invalid_request_with_meta(
                format!("Symbol '{}' not found", request.to),
                meta.clone(),
                None,
                Vec::new(),
            ));
        };

        // Find path
        let path_with_edges = graph.find_path_with_edges(from_node, to_node);

        path_with_edges.map_or_else(
            || (false, Vec::new(), 0),
            |path| {
                let steps: Vec<TraceStep> = path
                    .iter()
                    .map(|(n, rel)| {
                        let node_data = graph.get_node(*n);
                        let (symbol, file, line) = node_data.map_or_else(
                            || (String::new(), String::new(), 0),
                            |nd| {
                                (
                                    nd.symbol.name.clone(),
                                    nd.symbol.file_path.clone(),
                                    nd.symbol.start_line,
                                )
                            },
                        );
                        TraceStep {
                            symbol,
                            file,
                            line,
                            relationship: rel.map(|r| format!("{r:?}")),
                        }
                    })
                    .collect();
                let depth = steps.len().saturating_sub(1);
                (true, steps, depth)
            },
        )
    };

    drop(engine);

    // Generate Mermaid sequence diagram
    let mermaid = ContextFinderService::generate_trace_mermaid(&path_steps);

    let result = TraceResult {
        found,
        path: path_steps,
        depth,
        mermaid,
        meta,
    };

    Ok(CallToolResult::success(vec![Content::text(
        context_protocol::serialize_json(&result).unwrap_or_default(),
    )]))
}
