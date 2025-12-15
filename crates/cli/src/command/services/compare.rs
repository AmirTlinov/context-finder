use super::search::{
    collect_chunks, dedup_results, format_basic_output, format_enriched_output, key_for,
    overlap_ratio, parse_graph_language,
};
use crate::command::context::{
    ensure_index_exists, index_path, load_store_mtime, unix_ms, CommandContext,
};
use crate::command::domain::{
    config_bool_path, config_string_path, config_usize_path, parse_payload, CommandOutcome,
    CompareSearchPayload, ComparisonOutput, ComparisonSummary, Hint, HintKind, QueryComparison,
    SearchStrategy,
};
use crate::command::infra::{CompareCacheAdapter, GraphCacheFactory, HealthPort};
use crate::command::warm;
use anyhow::{Context as AnyhowContext, Result};
use context_graph::GraphLanguage;
use context_search::ContextSearch;
use context_search::HybridSearch;
use context_vector_store::VectorStore;
use log::warn;
use serde_json::Value;
use std::collections::HashSet;

pub struct CompareService {
    cache: CompareCacheAdapter,
    graph: GraphCacheFactory,
    health: HealthPort,
}

impl CompareService {
    pub fn new(cache: CompareCacheAdapter, graph: GraphCacheFactory, health: HealthPort) -> Self {
        Self {
            cache,
            graph,
            health,
        }
    }

    pub async fn run(&self, payload: Value, ctx: &CommandContext) -> Result<CommandOutcome> {
        let payload: CompareSearchPayload = parse_payload(payload)?;
        let mut queries = if payload.queries.is_empty() {
            payload.query.clone().into_iter().collect::<Vec<_>>()
        } else {
            payload.queries.clone()
        };
        if queries.is_empty() {
            anyhow::bail!("compare_search requires at least one query");
        }

        let project_ctx = ctx.resolve_project(payload.project).await?;
        let _ = crate::heartbeat::ping(&project_ctx.root).await;
        let warm = warm::global_warmer().prewarm(&project_ctx.root).await;
        let limit = payload
            .limit
            .or_else(|| config_usize_path(&project_ctx.config, &["defaults", "compare", "limit"]))
            .or_else(|| {
                config_usize_path(
                    &project_ctx.config,
                    &["defaults", "search_with_context", "limit"],
                )
            })
            .unwrap_or(crate::command::domain::DEFAULT_LIMIT);
        let strategy = payload
            .strategy
            .or_else(|| {
                config_string_path(&project_ctx.config, &["defaults", "compare", "strategy"])
                    .or_else(|| {
                        config_string_path(
                            &project_ctx.config,
                            &["defaults", "search_with_context", "strategy"],
                        )
                    })
                    .and_then(|value| SearchStrategy::from_name(&value))
            })
            .unwrap_or_default();
        let show_graph = payload
            .show_graph
            .or_else(|| {
                config_bool_path(&project_ctx.config, &["defaults", "compare", "show_graph"])
            })
            .or_else(|| {
                config_bool_path(
                    &project_ctx.config,
                    &["defaults", "search_with_context", "show_graph"],
                )
            })
            .unwrap_or(false);
        let reuse_graph = payload
            .reuse_graph
            .or_else(|| {
                config_bool_path(&project_ctx.config, &["defaults", "compare", "reuse_graph"])
            })
            .or_else(|| {
                config_bool_path(
                    &project_ctx.config,
                    &["defaults", "search_with_context", "reuse_graph"],
                )
            })
            .unwrap_or(true);
        let invalidate_cache = payload.invalidate_cache.unwrap_or(false);

        let language_pref = payload.language.clone().or_else(|| {
            config_string_path(&project_ctx.config, &["defaults", "compare", "language"])
                .or_else(|| {
                    config_string_path(
                        &project_ctx.config,
                        &["defaults", "search_with_context", "language"],
                    )
                })
                .or_else(|| {
                    crate::command::context::graph_language_from_config(&project_ctx.config)
                })
        });
        let language = language_pref
            .as_deref()
            .map(parse_graph_language)
            .transpose()?
            .unwrap_or(GraphLanguage::Rust);

        let store_path = index_path(&project_ctx.root);
        ensure_index_exists(&store_path)?;
        let store_mtime = load_store_mtime(&store_path).await?;
        let store_mtime_ms = unix_ms(store_mtime);
        let index_size_bytes = tokio::fs::metadata(&store_path).await.ok().map(|m| m.len());
        let profile = project_ctx.profile.clone();

        let cache_key = self.cache.key(
            &project_ctx.root,
            &queries,
            limit,
            strategy.as_str(),
            reuse_graph,
            show_graph,
            language_pref.as_deref().unwrap_or("rust"),
            store_mtime_ms,
        );

        if !invalidate_cache {
            if let Some(cached) = self
                .cache
                .load(&cache_key, store_mtime_ms)
                .await
                .ok()
                .flatten()
            {
                let mut outcome = CommandOutcome::from_value(cached)?;
                outcome.meta.config_path = project_ctx.config_path;
                outcome.meta.profile = Some(project_ctx.profile_name.clone());
                outcome.meta.profile_path = project_ctx.profile_path.clone();
                outcome.meta.index_updated = Some(false);
                outcome.meta.graph_cache = None;
                outcome.meta.index_mtime_ms = Some(store_mtime_ms);
                outcome.hints.extend(project_ctx.hints);
                outcome.hints.push(Hint {
                    kind: HintKind::Cache,
                    text: format!("compare_search cache hit ({cache_key})"),
                });
                self.health.attach(&project_ctx.root, &mut outcome).await;
                return Ok(outcome);
            }
        }

        let store_baseline = VectorStore::load(&store_path)
            .await
            .context("Failed to load vector store for baseline")?;
        let (baseline_chunks, _) = collect_chunks(&store_baseline);
        let mut baseline_search =
            HybridSearch::with_profile(store_baseline, baseline_chunks, profile.clone())
                .context("Failed to init baseline search")?;

        let store_context = VectorStore::load(&store_path)
            .await
            .context("Failed to load vector store for context")?;
        let (context_chunks, chunk_lookup) = collect_chunks(&store_context);

        let graph_cache = self.graph.for_root(&project_ctx.root);
        let mut graph_cache_used = false;
        let cached_assembler = if reuse_graph {
            graph_cache
                .load(store_mtime, language, &context_chunks, &chunk_lookup)
                .await?
        } else {
            None
        };

        let hybrid = HybridSearch::with_profile(store_context, context_chunks, profile.clone())
            .context("Failed to init context search")?;
        let mut context_search =
            ContextSearch::new(hybrid).context("Failed to create compare context search")?;

        if let Some(assembler) = cached_assembler {
            context_search.set_assembler(assembler);
            graph_cache_used = true;
        }
        if context_search.assembler().is_none() {
            context_search
                .build_graph(language)
                .context("Failed to build graph for compare")?;
            if reuse_graph {
                if let Some(assembler) = context_search.assembler() {
                    if let Err(err) = graph_cache.save(store_mtime, language, assembler).await {
                        warn!("Failed to store graph cache: {err}");
                    }
                }
            }
        }

        if queries.len() == 1 {
            queries.shrink_to_fit();
        }

        let mut comparison_rows = Vec::with_capacity(queries.len());
        let mut baseline_total_ms = 0u64;
        let mut context_total_ms = 0u64;
        let mut overlap_sum = 0f32;
        let mut related_sum = 0f32;
        let mut total_dropped = 0usize;

        for query in &queries {
            let baseline_start = std::time::Instant::now();
            let baseline_results = baseline_search
                .search(query, limit)
                .await
                .context("Baseline search failed")?;
            let baseline_duration_ms = baseline_start.elapsed().as_millis() as u64;

            let context_start = std::time::Instant::now();
            let enriched_results = context_search
                .search_with_context(query, limit, strategy.to_assembly())
                .await
                .context("Context search failed")?;
            let context_duration_ms = context_start.elapsed().as_millis() as u64;

            let baseline_outputs: Vec<_> = baseline_results
                .clone()
                .into_iter()
                .map(format_basic_output)
                .collect();
            let (baseline_outputs, dup_base) = dedup_results(baseline_outputs, &profile);
            let context_related_total: usize =
                enriched_results.iter().map(|er| er.related.len()).sum();
            let context_outputs: Vec<_> = enriched_results
                .into_iter()
                .map(|er| format_enriched_output(er, show_graph, &profile))
                .collect();
            let (context_outputs, dup_ctx) = dedup_results(context_outputs, &profile);
            if dup_base + dup_ctx > 0 {
                total_dropped += dup_base + dup_ctx;
            }

            let baseline_keys: HashSet<_> = baseline_outputs.iter().map(key_for).collect();
            let context_keys: HashSet<_> = context_outputs.iter().map(key_for).collect();
            let overlap_ratio = overlap_ratio(limit, &baseline_keys, &context_keys);

            baseline_total_ms += baseline_duration_ms;
            context_total_ms += context_duration_ms;
            overlap_sum += overlap_ratio;
            related_sum += context_related_total as f32;

            comparison_rows.push(QueryComparison {
                query: query.clone(),
                limit,
                baseline_duration_ms,
                context_duration_ms,
                overlap: baseline_keys.intersection(&context_keys).count(),
                overlap_ratio,
                context_related: context_related_total,
                baseline: baseline_outputs,
                context: context_outputs,
            });
        }

        let denom = queries.len() as f32;
        let summary = ComparisonSummary {
            avg_baseline_ms: baseline_total_ms as f32 / denom,
            avg_context_ms: context_total_ms as f32 / denom,
            avg_overlap_ratio: overlap_sum / denom,
            avg_related_chunks: if denom > 0.0 {
                related_sum / denom
            } else {
                0.0
            },
        };
        let summary_for_meta = summary.clone();
        let summary_hint = format!(
            "Baseline avg {:.1} ms vs context {:.1} ms (overlap {:.0}% per query, related +{:.1}/q)",
            summary.avg_baseline_ms,
            summary.avg_context_ms,
            summary.avg_overlap_ratio * 100.0,
            summary.avg_related_chunks
        );

        let output = ComparisonOutput {
            project: project_ctx.root.display().to_string(),
            limit,
            strategy: strategy.as_str().to_string(),
            reuse_graph,
            queries: comparison_rows,
            summary,
        };

        let output_for_cache = output.clone();
        let mut outcome = CommandOutcome::from_value(output)?;
        outcome.meta.config_path = project_ctx.config_path;
        outcome.meta.profile = Some(project_ctx.profile_name.clone());
        outcome.meta.profile_path = project_ctx.profile_path.clone();
        outcome.meta.index_updated = Some(false);
        outcome.meta.graph_cache = Some(graph_cache_used);
        if graph_cache_used {
            outcome.hints.push(Hint {
                kind: HintKind::Cache,
                text: "Graph cache hit (compare_search)".to_string(),
            });
        }
        outcome.meta.index_mtime_ms = Some(store_mtime_ms);
        outcome.meta.index_size_bytes = index_size_bytes;
        outcome.meta.warm = Some(warm.warmed);
        outcome.meta.warm_cost_ms = Some(warm.warm_cost_ms);
        outcome.meta.warm_graph_cache_hit = Some(warm.graph_cache_hit);
        outcome.meta.compare_avg_baseline_ms = Some(summary_for_meta.avg_baseline_ms);
        outcome.meta.compare_avg_context_ms = Some(summary_for_meta.avg_context_ms);
        outcome.meta.compare_avg_overlap_ratio = Some(summary_for_meta.avg_overlap_ratio);
        outcome.meta.compare_avg_related = Some(summary_for_meta.avg_related_chunks);
        if let Some((nodes, edges)) = context_search.graph_stats() {
            outcome.meta.graph_nodes = Some(nodes);
            outcome.meta.graph_edges = Some(edges);
        }
        if total_dropped > 0 {
            outcome.meta.duplicates_dropped = Some(total_dropped);
            outcome.hints.push(Hint {
                kind: HintKind::Info,
                text: format!(
                    "Deduplicated {total_dropped} overlapping results across baseline/context"
                ),
            });
        }
        outcome.meta.graph_cache_size_bytes = graph_cache.size_bytes().await;
        outcome.hints.extend(project_ctx.hints);
        if invalidate_cache {
            outcome.hints.push(Hint {
                kind: HintKind::Action,
                text: "compare_search cache invalidated for this run".to_string(),
            });
        }
        outcome.hints.push(Hint {
            kind: HintKind::Info,
            text: summary_hint,
        });
        let _ = self
            .cache
            .save(&cache_key, store_mtime_ms, &output_for_cache)
            .await;
        self.health.attach(&project_ctx.root, &mut outcome).await;
        Ok(outcome)
    }
}
