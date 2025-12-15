use crate::command::context::CommandContext;
use crate::command::domain::{
    parse_payload, CommandOutcome, ContextOutput, GetContextPayload, ListSymbolsPayload, MapNode,
    MapOutput, MapPayload, SymbolInfo, SymbolsOutput, WindowOutput,
};
use crate::command::warm;
use anyhow::{anyhow, Context as AnyhowContext, Result};
use context_code_chunker::{Chunker, ChunkerConfig};
use std::collections::{HashMap, HashSet};
use tokio::fs;

#[derive(Debug, Clone)]
struct SymAgg {
    files: HashMap<String, usize>, // file -> max lines for this symbol in file
    symbol_type: String,
    parent: Option<String>,
    line: usize,
}

#[derive(Default)]
pub struct ContextService;

impl ContextService {
    pub async fn get(
        &self,
        payload: serde_json::Value,
        ctx: &CommandContext,
    ) -> Result<CommandOutcome> {
        let payload: GetContextPayload = parse_payload(payload)?;
        let project_ctx = ctx.resolve_project(payload.project).await?;
        let _ = crate::heartbeat::ping(&project_ctx.root).await;
        let warm = warm::global_warmer().prewarm(&project_ctx.root).await;
        let file_path = project_ctx.root.join(&payload.file);

        if !file_path.exists() {
            return Err(anyhow!("File not found: {}", file_path.display()));
        }

        let content = fs::read_to_string(&file_path)
            .await
            .context("Failed to read file")?;
        let lines: Vec<&str> = content.lines().collect();

        if payload.line == 0 || payload.line > lines.len() {
            return Err(anyhow!(
                "Line {} out of range (file has {} lines)",
                payload.line,
                lines.len()
            ));
        }

        let chunker = Chunker::new(ChunkerConfig::for_embeddings());
        let chunks = chunker
            .chunk_str(&content, Some(&payload.file))
            .context("Failed to chunk file")?;

        let target_chunk = chunks
            .iter()
            .find(|chunk| payload.line >= chunk.start_line && payload.line <= chunk.end_line);

        let before_lines = lines[..payload.line.saturating_sub(1)]
            .iter()
            .rev()
            .take(payload.window)
            .rev()
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        let after_lines = lines[payload.line..]
            .iter()
            .take(payload.window)
            .copied()
            .collect::<Vec<_>>()
            .join("\n");

        let output = ContextOutput {
            file: payload.file,
            line: payload.line,
            symbol: target_chunk.and_then(|c| c.metadata.symbol_name.clone()),
            chunk_type: target_chunk
                .and_then(|c| c.metadata.chunk_type.map(|ct| ct.as_str().to_string())),
            parent: target_chunk.and_then(|c| c.metadata.parent_scope.clone()),
            imports: target_chunk
                .map(|c| c.metadata.context_imports.clone())
                .unwrap_or_default(),
            content: target_chunk.map(|c| c.content.clone()).unwrap_or_default(),
            window: WindowOutput {
                before: before_lines,
                after: after_lines,
            },
        };

        let mut outcome = CommandOutcome::from_value(output)?;
        outcome.meta.config_path = project_ctx.config_path;
        outcome.meta.index_updated = Some(false);
        outcome.meta.warm = Some(warm.warmed);
        outcome.meta.warm_cost_ms = Some(warm.warm_cost_ms);
        outcome.meta.warm_graph_cache_hit = Some(warm.graph_cache_hit);
        outcome.hints.extend(project_ctx.hints);
        Ok(outcome)
    }

    pub async fn list_symbols(
        &self,
        payload: serde_json::Value,
        ctx: &CommandContext,
    ) -> Result<CommandOutcome> {
        let payload: ListSymbolsPayload = parse_payload(payload)?;
        let project_ctx = ctx.resolve_project(payload.project).await?;
        let _ = crate::heartbeat::ping(&project_ctx.root).await;
        let warm = warm::global_warmer().prewarm(&project_ctx.root).await;

        let file_pattern = &payload.file;
        let is_all_files = file_pattern == "*" || file_pattern.is_empty();
        let is_glob = file_pattern.contains('*') || file_pattern.contains('?');

        // Use index for glob/all-files mode (much faster)
        if is_all_files || is_glob {
            let store_path = crate::command::context::index_path(&project_ctx.root);
            crate::command::context::ensure_index_exists(&store_path)?;
            let store = context_vector_store::VectorStore::load(&store_path).await?;

            let glob_matcher = if is_glob && !is_all_files {
                Some(glob::Pattern::new(file_pattern).context("Invalid glob pattern")?)
            } else {
                None
            };

            let mut symbols: Vec<SymbolInfo> = Vec::new();
            let mut files_seen: HashSet<String> = HashSet::new();

            for id in store.chunk_ids() {
                if let Some(chunk) = store.get_chunk(&id) {
                    let file_path = &chunk.chunk.file_path;

                    // Apply glob filter if specified
                    if let Some(ref matcher) = glob_matcher {
                        if !matcher.matches(file_path) {
                            continue;
                        }
                    }

                    files_seen.insert(file_path.clone());

                    if let Some(name) = &chunk.chunk.metadata.symbol_name {
                        let symbol_type = chunk
                            .chunk
                            .metadata
                            .chunk_type
                            .map(|ct| ct.as_str().to_string())
                            .unwrap_or_else(|| "unknown".to_string());

                        symbols.push(SymbolInfo {
                            name: name.clone(),
                            symbol_type,
                            parent: chunk.chunk.metadata.parent_scope.clone(),
                            line: chunk.chunk.start_line,
                            file: Some(file_path.clone()),
                        });
                    }
                }
            }

            // Sort by file then line
            symbols.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));

            let output = SymbolsOutput {
                file: file_pattern.clone(),
                symbols,
                files_count: Some(files_seen.len()),
            };

            let mut outcome = CommandOutcome::from_value(output)?;
            outcome.meta.config_path = project_ctx.config_path;
            outcome.meta.index_updated = Some(false);
            outcome.meta.warm = Some(warm.warmed);
            outcome.meta.warm_cost_ms = Some(warm.warm_cost_ms);
            outcome.meta.warm_graph_cache_hit = Some(warm.graph_cache_hit);
            outcome.hints.extend(project_ctx.hints);
            return Ok(outcome);
        }

        // Single-file mode: read and parse the file directly
        let file_path = project_ctx.root.join(&payload.file);

        if !file_path.exists() {
            return Err(anyhow!("File not found: {}", file_path.display()));
        }

        let content = fs::read_to_string(&file_path)
            .await
            .context("Failed to read file")?;

        let chunker = Chunker::new(ChunkerConfig::for_embeddings());
        let chunks = chunker
            .chunk_str(&content, Some(&payload.file))
            .context("Failed to chunk file")?;

        let symbols: Vec<SymbolInfo> = chunks
            .iter()
            .filter_map(|chunk| {
                let name = chunk.metadata.symbol_name.clone()?;
                let symbol_type = chunk
                    .metadata
                    .chunk_type
                    .map_or_else(|| "unknown".to_string(), |ct| ct.as_str().to_string());

                Some(SymbolInfo {
                    name,
                    symbol_type,
                    parent: chunk.metadata.parent_scope.clone(),
                    line: chunk.start_line,
                    file: None,
                })
            })
            .collect();

        let output = SymbolsOutput {
            file: payload.file,
            symbols,
            files_count: None,
        };

        let mut outcome = CommandOutcome::from_value(output)?;
        outcome.meta.config_path = project_ctx.config_path;
        outcome.meta.index_updated = Some(false);
        outcome.meta.warm = Some(warm.warmed);
        outcome.meta.warm_cost_ms = Some(warm.warm_cost_ms);
        outcome.meta.warm_graph_cache_hit = Some(warm.graph_cache_hit);
        outcome.hints.extend(project_ctx.hints);
        Ok(outcome)
    }

    pub async fn map(
        &self,
        payload: serde_json::Value,
        ctx: &CommandContext,
    ) -> Result<CommandOutcome> {
        let payload: MapPayload = parse_payload(payload)?;
        let project_ctx = ctx.resolve_project(payload.project).await?;
        let _ = crate::heartbeat::ping(&project_ctx.root).await;
        let warm = warm::global_warmer().prewarm(&project_ctx.root).await;

        let store_path = crate::command::context::index_path(&project_ctx.root);
        crate::command::context::ensure_index_exists(&store_path)?;
        let store = context_vector_store::VectorStore::load(&store_path).await?;

        // Aggregate by top-level path up to depth
        let mut tree_files: HashMap<String, HashSet<String>> = HashMap::new();
        let mut tree_chunks: HashMap<String, usize> = HashMap::new();
        let mut tree_symbols: HashMap<String, HashMap<String, SymAgg>> = HashMap::new();
        let mut tree_lines: HashMap<String, usize> = HashMap::new();
        let mut total_lines: usize = 0;
        let mut all_files: HashSet<String> = HashSet::new();
        let mut file_lines: HashMap<String, usize> = HashMap::new();
        for id in store.chunk_ids() {
            if let Some(chunk) = store.get_chunk(&id) {
                let parts: Vec<&str> = chunk.chunk.file_path.split('/').collect();
                let key = parts
                    .iter()
                    .take(payload.depth)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("/");
                tree_chunks
                    .entry(key.clone())
                    .and_modify(|c| *c += 1)
                    .or_insert(1);
                tree_files
                    .entry(key.clone())
                    .or_default()
                    .insert(chunk.chunk.file_path.clone());
                all_files.insert(chunk.chunk.file_path.clone());
                let lines = chunk.chunk.content.lines().count().max(1);
                total_lines += lines;
                tree_lines
                    .entry(key.clone())
                    .and_modify(|v| *v += lines)
                    .or_insert(lines);
                file_lines
                    .entry(chunk.chunk.file_path.clone())
                    .and_modify(|v| *v = (*v).max(lines))
                    .or_insert(lines);
                if let Some(sym) = &chunk.chunk.metadata.symbol_name {
                    let sym_type = chunk
                        .chunk
                        .metadata
                        .chunk_type
                        .map(|ct| ct.as_str().to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    let sym_map = tree_symbols.entry(key).or_default();
                    sym_map
                        .entry(sym.clone())
                        .and_modify(|agg| {
                            let entry = agg.files.entry(chunk.chunk.file_path.clone()).or_insert(0);
                            *entry = (*entry).max(lines);
                            agg.line = agg.line.min(chunk.chunk.start_line);
                        })
                        .or_insert(SymAgg {
                            files: {
                                let mut m = HashMap::new();
                                m.insert(chunk.chunk.file_path.clone(), lines);
                                m
                            },
                            symbol_type: sym_type,
                            parent: chunk.chunk.metadata.parent_scope.clone(),
                            line: chunk.chunk.start_line,
                        });
                }
            }
        }

        let mut nodes: Vec<MapNode> = tree_chunks
            .into_iter()
            .map(|(path, chunks)| MapNode {
                path: path.clone(),
                files: tree_files.get(&path).map(|s| s.len()).unwrap_or(0),
                chunks,
                coverage_chunks_pct: None,
                coverage_files_pct: None,
                coverage_lines_pct: None,
                top_symbols: tree_symbols
                    .get(&path)
                    .map(|m| top_symbols(m, 5, &file_lines)),
                avg_symbol_coverage: None,
            })
            .collect();
        nodes.sort_by(|a, b| b.chunks.cmp(&a.chunks));
        if let Some(limit) = payload.limit {
            nodes.truncate(limit);
        }

        let total_chunks = store.chunk_ids().len();
        let total_files = all_files.len();
        let coverage_chunks_pct = if total_chunks > 0 {
            Some(nodes.iter().map(|n| n.chunks).sum::<usize>() as f32 / total_chunks as f32 * 100.0)
        } else {
            None
        };
        let coverage_files_pct = if total_files > 0 {
            Some(nodes.iter().map(|n| n.files).sum::<usize>() as f32 / total_files as f32 * 100.0)
        } else {
            None
        };
        let coverage_lines_pct = if total_lines > 0 {
            let lines_kept: usize = nodes
                .iter()
                .map(|n| tree_lines.get(&n.path).copied().unwrap_or(0))
                .sum();
            Some(lines_kept as f32 / total_lines as f32 * 100.0)
        } else {
            None
        };
        for node in nodes.iter_mut() {
            if total_chunks > 0 {
                node.coverage_chunks_pct = Some(node.chunks as f32 / total_chunks as f32 * 100.0);
            }
            if total_files > 0 {
                node.coverage_files_pct = Some(node.files as f32 / total_files as f32 * 100.0);
            }
            if total_lines > 0 {
                let lines = tree_lines.get(&node.path).copied().unwrap_or(0);
                node.coverage_lines_pct = Some(lines as f32 / total_lines as f32 * 100.0);
            }
            if let Some(sym_list) = node.top_symbols.as_ref() {
                if total_lines > 0 {
                    let mut acc = 0.0f32;
                    let mut count = 0usize;
                    if let Some(map) = tree_symbols.get(&node.path) {
                        for s in sym_list {
                            if let Some(agg) = map.get(&s.name) {
                                let score = symbol_score(agg, &file_lines);
                                acc += score * 100.0;
                                count += 1;
                            }
                        }
                    }
                    if count > 0 {
                        node.avg_symbol_coverage = Some(acc / count as f32);
                    }
                }
            }
        }

        let output = MapOutput {
            nodes,
            total_files,
            total_chunks,
            coverage_chunks_pct,
            total_lines: Some(total_lines),
            coverage_files_pct,
            coverage_lines_pct,
        };

        let mut outcome = CommandOutcome::from_value(output)?;
        outcome.meta.config_path = project_ctx.config_path;
        outcome.meta.index_updated = Some(false);
        outcome.meta.index_size_bytes =
            tokio::fs::metadata(&store_path).await.ok().map(|m| m.len());
        outcome.meta.warm = Some(warm.warmed);
        outcome.meta.warm_cost_ms = Some(warm.warm_cost_ms);
        outcome.meta.warm_graph_cache_hit = Some(warm.graph_cache_hit);
        outcome.meta.duplicates_dropped = None;
        outcome.hints.extend(project_ctx.hints);
        outcome.hints.push(crate::command::domain::Hint {
            kind: crate::command::domain::HintKind::Info,
            text: "Map generated from existing index (no extra work)".to_string(),
        });
        Ok(outcome)
    }
}

fn top_symbols(
    counts: &std::collections::HashMap<String, SymAgg>,
    limit: usize,
    file_lines: &HashMap<String, usize>,
) -> Vec<SymbolInfo> {
    let mut items: Vec<(&String, &SymAgg)> = counts.iter().collect();
    items.sort_by(|a, b| {
        let a_score = symbol_score(a.1, file_lines);
        let b_score = symbol_score(b.1, file_lines);
        b_score
            .partial_cmp(&a_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(b.0))
    });
    items.truncate(limit);
    items
        .into_iter()
        .map(|(name, agg)| SymbolInfo {
            name: name.clone(),
            symbol_type: agg.symbol_type.clone(),
            parent: agg.parent.clone(),
            line: agg.line,
            file: None,
        })
        .collect()
}

fn symbol_score(agg: &SymAgg, file_lines: &HashMap<String, usize>) -> f32 {
    let mut score = 0f32;
    for (file, sym_lines) in &agg.files {
        let flines = *file_lines.get(file).unwrap_or(sym_lines);
        if flines == 0 {
            continue;
        }
        let covered = (*sym_lines).min(flines) as f32;
        score += covered / flines as f32; // sum per-file coverage share, no double counting
    }
    score
}
