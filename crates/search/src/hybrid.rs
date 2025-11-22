use crate::error::{Result, SearchError};
use crate::fusion::{AstBooster, RRFFusion};
use crate::fuzzy::FuzzySearch;
use crate::profile::SearchProfile;
use crate::query_classifier::{QueryClassifier, QueryWeights};
use crate::query_expansion::QueryExpander;
use crate::rerank::rerank_candidates;
use context_code_chunker::CodeChunk;
use context_vector_store::{SearchResult, VectorStore};
use std::collections::HashMap;
/// Hybrid search combining semantic, fuzzy, and RRF fusion
pub struct HybridSearch {
    store: VectorStore,
    chunks: Vec<CodeChunk>,
    fuzzy: FuzzySearch,
    fusion: RRFFusion,
    expander: QueryExpander,
    profile: SearchProfile,
}

impl HybridSearch {
    /// Create new hybrid search engine
    pub fn new(store: VectorStore, chunks: Vec<CodeChunk>) -> Result<Self> {
        Self::with_profile(store, chunks, SearchProfile::general())
    }

    /// Create hybrid search engine with explicit profile
    pub fn with_profile(
        store: VectorStore,
        chunks: Vec<CodeChunk>,
        profile: SearchProfile,
    ) -> Result<Self> {
        Ok(Self {
            store,
            chunks,
            fuzzy: FuzzySearch::new(),
            fusion: RRFFusion::default(),
            expander: QueryExpander::new(),
            profile,
        })
    }
    /// Search with full hybrid strategy: semantic + fuzzy + RRF + AST boost
    pub async fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        if query.trim().is_empty() {
            return Err(SearchError::EmptyQuery);
        }

        log::debug!("Hybrid search: query='{query}', limit={limit}");

        // Expand query with synonyms and variants
        let expanded_query = self.expander.expand_to_query(query);
        log::debug!("Expanded query: '{expanded_query}'");

        let weights = QueryClassifier::weights(query);
        let candidate_pool = Self::candidate_pool(limit, weights.candidate_multiplier);
        let tokens = query_tokens(query);

        // Build chunk id -> index mapping
        let mut chunk_id_to_idx: HashMap<String, usize> = HashMap::new();
        let rejected: Vec<bool> = self
            .chunks
            .iter()
            .map(|c| self.profile.is_rejected(&c.file_path))
            .collect();
        for (idx, chunk) in self.chunks.iter().enumerate() {
            let id = format!(
                "{}:{}:{}",
                chunk.file_path, chunk.start_line, chunk.end_line
            );
            chunk_id_to_idx.insert(id, idx);
        }

        // 1. Semantic search (embeddings + cosine similarity) with expanded query
        let semantic_results = self.store.search(&expanded_query, candidate_pool).await?;
        log::debug!("Semantic: {} results", semantic_results.len());

        // Convert semantic results to (chunk_idx, score) using chunk_id_to_idx
        let semantic_scores: Vec<(usize, f32)> = semantic_results
            .iter()
            .filter_map(|result| {
                chunk_id_to_idx
                    .get(&result.id)
                    .and_then(|&idx| (!rejected[idx]).then_some((idx, result.score)))
            })
            .collect();
        let semantic_map: HashMap<usize, f32> = semantic_scores.iter().copied().collect();

        // 2. Fuzzy search (path/symbol matching)
        let min_fuzzy = self.profile.min_fuzzy_score();
        let fuzzy_scores = Self::filter_fuzzy(
            self.fuzzy.search(query, &self.chunks, candidate_pool),
            &rejected,
            min_fuzzy,
        );
        let fuzzy_map: HashMap<usize, f32> = fuzzy_scores.iter().copied().collect();
        log::debug!("Fuzzy: {} results", fuzzy_scores.len());

        // 3. RRF Fusion with adaptive weights based on query type
        let fused_scores =
            self.fusion
                .fuse_adaptive(query, &weights, &semantic_scores, &fuzzy_scores);
        log::debug!("Fused: {} results", fused_scores.len());

        // 4. AST-aware boosting + rule-based rerank
        let boosted_scores = rerank_candidates(
            &self.profile,
            &self.chunks,
            &tokens,
            AstBooster::boost(&self.chunks, fused_scores),
            &semantic_map,
            &fuzzy_map,
        );

        // 5. Convert back to SearchResult using chunk indices
        let mut final_results: Vec<SearchResult> = boosted_scores
            .into_iter()
            .filter_map(|(idx, score)| {
                self.chunks.get(idx).map(|chunk| {
                    let id = format!(
                        "{}:{}:{}",
                        chunk.file_path, chunk.start_line, chunk.end_line
                    );
                    let penalized = score * self.profile.path_weight(&chunk.file_path);
                    SearchResult {
                        chunk: chunk.clone(),
                        score: penalized,
                        id,
                    }
                })
            })
            .collect();

        // 6. Normalize scores to 0-1 range
        Self::normalize_scores(&mut final_results);

        // Sort by final score descending
        final_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        final_results.truncate(limit);

        log::info!(
            "Hybrid search completed: {} final results",
            final_results.len()
        );

        Ok(final_results)
    }

    /// Batch search for multiple queries (more efficient than sequential searches)
    /// Returns results for each query in the same order
    pub async fn search_batch(
        &mut self,
        queries: &[&str],
        limit: usize,
    ) -> Result<Vec<Vec<SearchResult>>> {
        if queries.is_empty() {
            return Ok(vec![]);
        }

        // Check for empty queries
        for query in queries {
            if query.trim().is_empty() {
                return Err(SearchError::EmptyQuery);
            }
        }

        log::debug!(
            "Batch hybrid search: {} queries, limit={}",
            queries.len(),
            limit
        );

        let query_weights: Vec<QueryWeights> = queries
            .iter()
            .map(|q| QueryClassifier::weights(q))
            .collect();
        let tokens: Vec<Vec<String>> = queries.iter().map(|q| query_tokens(q)).collect();
        let max_multiplier = query_weights
            .iter()
            .map(|w| w.candidate_multiplier)
            .max()
            .unwrap_or(5);
        let candidate_pool = Self::candidate_pool(limit, max_multiplier);

        // Build chunk id -> index mapping (once for all queries)
        let mut chunk_id_to_idx: HashMap<String, usize> = HashMap::new();
        let rejected: Vec<bool> = self
            .chunks
            .iter()
            .map(|c| self.profile.is_rejected(&c.file_path))
            .collect();
        for (idx, chunk) in self.chunks.iter().enumerate() {
            let id = format!(
                "{}:{}:{}",
                chunk.file_path, chunk.start_line, chunk.end_line
            );
            chunk_id_to_idx.insert(id, idx);
        }

        // 1. Expand all queries
        let expanded_queries: Vec<String> = queries
            .iter()
            .map(|q| self.expander.expand_to_query(q))
            .collect();

        // 2. Batch semantic search with expanded queries
        let expanded_refs: Vec<&str> = expanded_queries
            .iter()
            .map(std::string::String::as_str)
            .collect();
        let semantic_results_batch = self
            .store
            .search_batch(&expanded_refs, candidate_pool)
            .await?;
        log::debug!(
            "Semantic batch: {} queries processed",
            semantic_results_batch.len()
        );

        // 3. Process each query: fuzzy + RRF + AST boost
        let mut all_final_results = Vec::with_capacity(queries.len());
        for (i, query) in queries.iter().enumerate() {
            let semantic_results = &semantic_results_batch[i];
            let weights = query_weights[i];

            // Convert semantic results to (chunk_idx, score)
            let semantic_scores: Vec<(usize, f32)> = semantic_results
                .iter()
                .filter_map(|result| {
                    chunk_id_to_idx
                        .get(&result.id)
                        .and_then(|&idx| (!rejected[idx]).then_some((idx, result.score)))
                })
                .collect();
            let semantic_map: HashMap<usize, f32> = semantic_scores.iter().copied().collect();

            // Fuzzy search for this query
            let min_fuzzy = self.profile.min_fuzzy_score();
            let fuzzy_scores = Self::filter_fuzzy(
                self.fuzzy.search(query, &self.chunks, candidate_pool),
                &rejected,
                min_fuzzy,
            );
            let fuzzy_map: HashMap<usize, f32> = fuzzy_scores.iter().copied().collect();

            // RRF Fusion with adaptive weights
            let fused_scores =
                self.fusion
                    .fuse_adaptive(query, &weights, &semantic_scores, &fuzzy_scores);

            // AST-aware boosting + rerank
            let boosted_scores = rerank_candidates(
                &self.profile,
                &self.chunks,
                &tokens[i],
                AstBooster::boost(&self.chunks, fused_scores),
                &semantic_map,
                &fuzzy_map,
            );

            // Convert to SearchResult
            let mut final_results: Vec<SearchResult> = boosted_scores
                .into_iter()
                .filter_map(|(idx, score)| {
                    self.chunks.get(idx).and_then(|chunk| {
                        has_query_overlap(chunk, &tokens[i]).then(|| {
                            let id = format!(
                                "{}:{}:{}",
                                chunk.file_path, chunk.start_line, chunk.end_line
                            );
                            let penalized = score * self.profile.path_weight(&chunk.file_path);
                            SearchResult {
                                chunk: chunk.clone(),
                                score: penalized,
                                id,
                            }
                        })
                    })
                })
                .collect();

            // Normalize scores to 0-1 range
            Self::normalize_scores(&mut final_results);

            // Sort and truncate
            final_results.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            final_results.truncate(limit);

            log::debug!(
                "Query {}/{}: {} final results",
                i + 1,
                queries.len(),
                final_results.len()
            );
            all_final_results.push(final_results);
        }

        log::info!("Batch hybrid search completed: {} queries", queries.len());
        Ok(all_final_results)
    }

    /// Semantic-only search (bypass fuzzy/fusion for speed)
    pub async fn search_semantic_only(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        if query.trim().is_empty() {
            return Err(SearchError::EmptyQuery);
        }

        self.store.search(query, limit).await.map_err(Into::into)
    }

    /// Get chunk by ID
    #[must_use]
    pub fn get_chunk(&self, id: &str) -> Option<&CodeChunk> {
        self.chunks.iter().find(|c| {
            let chunk_id = format!("{}:{}:{}", c.file_path, c.start_line, c.end_line);
            chunk_id == id
        })
    }

    /// Get all chunks
    #[must_use]
    pub fn chunks(&self) -> &[CodeChunk] {
        &self.chunks
    }

    /// Normalize scores to 0-1 range using min-max normalization
    pub fn normalize_scores(results: &mut [SearchResult]) {
        if results.is_empty() {
            return;
        }

        // Find min and max scores
        let mut min_score = f32::MAX;
        let mut max_score = f32::MIN;

        let mut had_invalid = false;
        for result in results.iter() {
            if result.score.is_finite() {
                min_score = min_score.min(result.score);
                max_score = max_score.max(result.score);
            } else {
                had_invalid = true;
            }
        }

        if !min_score.is_finite() || !max_score.is_finite() {
            for result in results {
                result.score = 0.0;
            }
            return;
        }

        if had_invalid && (max_score - min_score).abs() < f32::EPSILON {
            for result in results {
                if result.score.is_finite() {
                    result.score = 1.0;
                } else {
                    result.score = 0.0;
                }
            }
            return;
        }

        // Avoid division by zero if all scores are equal (allow tiny jitter)
        const MIN_DELTA: f32 = 1e-6;
        if (max_score - min_score).abs() < MIN_DELTA {
            // All scores are the same, set them all to 1.0
            for result in results {
                result.score = 1.0;
            }
            return;
        }

        // Replace remaining invalid scores with the minimum finite value
        for result in results.iter_mut() {
            if !result.score.is_finite() {
                log::warn!(
                    "Invalid score detected for {} — resetting to min",
                    result.id
                );
                result.score = min_score;
            }
        }

        // Normalize: (score - min) / (max - min)
        let range = max_score - min_score;
        for result in results {
            result.score = (result.score - min_score) / range;
        }

        log::debug!("Normalized scores: range [{min_score:.4}, {max_score:.4}] → [0.0, 1.0]");
    }

    fn candidate_pool(limit: usize, multiplier: usize) -> usize {
        let limit = limit.max(1);
        let multiplier = multiplier.max(4);
        limit * multiplier
    }

    fn filter_fuzzy(
        scores: Vec<(usize, f32)>,
        rejected: &[bool],
        min_score: f32,
    ) -> Vec<(usize, f32)> {
        scores
            .into_iter()
            .filter(|(idx, score)| {
                *score >= min_score && !rejected.get(*idx).copied().unwrap_or(false)
            })
            .collect()
    }
}

pub(crate) fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens: Vec<String> = query
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter_map(|raw| {
            let token = raw.trim().to_ascii_lowercase();
            if token.len() < 3 {
                None
            } else {
                Some(token)
            }
        })
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn has_query_overlap(chunk: &CodeChunk, tokens: &[String]) -> bool {
    if tokens.is_empty() {
        return true;
    }
    let mut haystacks = vec![
        chunk.file_path.to_ascii_lowercase(),
        chunk.content.to_ascii_lowercase(),
    ];
    if let Some(symbol) = &chunk.metadata.symbol_name {
        haystacks.push(symbol.to_ascii_lowercase());
    }

    tokens
        .iter()
        .any(|token| haystacks.iter().any(|hay| hay.contains(token)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use context_code_chunker::{ChunkMetadata, ChunkType};
    use tempfile::TempDir;

    fn create_test_chunk(path: &str, line: usize, symbol: &str, content: &str) -> CodeChunk {
        CodeChunk::new(
            path.to_string(),
            line,
            line + 10,
            content.to_string(),
            ChunkMetadata::default()
                .chunk_type(ChunkType::Function)
                .symbol_name(symbol),
        )
    }

    #[tokio::test]
    #[ignore = "Requires ONNX embedding model"]
    async fn test_hybrid_search() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("store.json");

        let chunks = vec![
            create_test_chunk(
                "api.rs",
                1,
                "handle_error",
                "async fn handle_error() { /* error handling */ }",
            ),
            create_test_chunk(
                "utils.rs",
                20,
                "parse_data",
                "fn parse_data(input: &str) -> Result<Data> {}",
            ),
            create_test_chunk("main.rs", 50, "main", "fn main() { println!(\"hello\"); }"),
        ];

        let mut store = VectorStore::new(&store_path).unwrap();
        store.add_chunks(chunks.clone()).await.unwrap();

        let mut search = HybridSearch::new(store, chunks).unwrap();

        let results = search.search("error handling", 5).await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    #[ignore = "Requires ONNX embedding model"]
    async fn test_batch_search() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("store.json");

        let chunks = vec![
            create_test_chunk(
                "api.rs",
                1,
                "handle_error",
                "async fn handle_error() { /* error handling */ }",
            ),
            create_test_chunk(
                "utils.rs",
                20,
                "parse_data",
                "fn parse_data(input: &str) -> Result<Data> {}",
            ),
            create_test_chunk("main.rs", 50, "main", "fn main() { println!(\"hello\"); }"),
            create_test_chunk(
                "db.rs",
                100,
                "query_db",
                "async fn query_db(sql: &str) -> Result<Vec<Row>> {}",
            ),
        ];

        let mut store = VectorStore::new(&store_path).unwrap();
        store.add_chunks(chunks.clone()).await.unwrap();

        let mut search = HybridSearch::new(store, chunks).unwrap();

        // Batch search with 3 queries
        let queries = vec!["error handling", "parse data", "database query"];
        let results_batch = search.search_batch(&queries, 5).await.unwrap();

        // Should return results for all 3 queries
        assert_eq!(results_batch.len(), 3);

        // Each query should have results
        for results in &results_batch {
            assert!(!results.is_empty());
        }

        // Verify order matches queries
        assert!(
            results_batch[0][0].chunk.content.contains("error")
                || results_batch[0][0].chunk.content.contains("handle_error")
        );
        assert!(
            results_batch[1][0].chunk.content.contains("parse")
                || results_batch[1][0].chunk.metadata.symbol_name.as_deref() == Some("parse_data")
        );
    }

    #[test]
    fn filters_by_query_overlap() {
        let chunk = create_test_chunk(
            "src/utils/selection_tables.rs",
            10,
            "create_selection_tables_handlers",
            "Selection tables helper functions",
        );
        let missing = create_test_chunk("src/app/page.tsx", 1, "page", "admin dashboard page");

        let tokens = query_tokens("selection tables helper");
        assert!(has_query_overlap(&chunk, &tokens));
        assert!(!has_query_overlap(&missing, &tokens));
    }
}
