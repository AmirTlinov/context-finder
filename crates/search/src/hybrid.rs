use crate::error::{Result, SearchError};
use crate::fusion::{AstBooster, RRFFusion};
use crate::fuzzy::FuzzySearch;
use crate::query_expansion::QueryExpander;
use context_code_chunker::CodeChunk;
use context_vector_store::{SearchResult, VectorStore};

/// Hybrid search combining semantic, fuzzy, and RRF fusion
pub struct HybridSearch {
    store: VectorStore,
    chunks: Vec<CodeChunk>,
    fuzzy: FuzzySearch,
    fusion: RRFFusion,
    expander: QueryExpander,
}

impl HybridSearch {
    /// Create new hybrid search engine
    pub async fn new(store: VectorStore, chunks: Vec<CodeChunk>) -> Result<Self> {
        Ok(Self {
            store,
            chunks,
            fuzzy: FuzzySearch::new(),
            fusion: RRFFusion::default(),
            expander: QueryExpander::new(),
        })
    }

    /// Search with full hybrid strategy: semantic + fuzzy + RRF + AST boost
    pub async fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        if query.trim().is_empty() {
            return Err(SearchError::EmptyQuery);
        }

        log::debug!("Hybrid search: query='{}', limit={}", query, limit);

        // Expand query with synonyms and variants
        let expanded_query = self.expander.expand_to_query(query);
        log::debug!("Expanded query: '{}'", expanded_query);

        // Candidate pool size (retrieve more for fusion)
        let candidate_pool = limit * 5;

        // Build chunk id -> index mapping
        let mut chunk_id_to_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (idx, chunk) in self.chunks.iter().enumerate() {
            let id = format!("{}:{}:{}", chunk.file_path, chunk.start_line, chunk.end_line);
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
                    .map(|&idx| (idx, result.score))
            })
            .collect();

        // 2. Fuzzy search (path/symbol matching)
        let fuzzy_scores = self.fuzzy.search(query, &self.chunks, candidate_pool);
        log::debug!("Fuzzy: {} results", fuzzy_scores.len());

        // 3. RRF Fusion with adaptive weights based on query type
        let fused_scores = self.fusion.fuse_adaptive(query, semantic_scores, fuzzy_scores);
        log::debug!("Fused: {} results", fused_scores.len());

        // 4. AST-aware boosting
        let boosted_scores = AstBooster::boost(&self.chunks, fused_scores);

        // 5. Convert back to SearchResult using chunk indices
        let mut final_results: Vec<SearchResult> = boosted_scores
            .into_iter()
            .filter_map(|(idx, score)| {
                self.chunks.get(idx).map(|chunk| {
                    let id = format!("{}:{}:{}", chunk.file_path, chunk.start_line, chunk.end_line);
                    SearchResult {
                        chunk: chunk.clone(),
                        score,
                        id,
                    }
                })
            })
            .collect();

        // Sort by final score descending
        final_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        final_results.truncate(limit);

        log::info!("Hybrid search completed: {} final results", final_results.len());

        Ok(final_results)
    }

    /// Batch search for multiple queries (more efficient than sequential searches)
    /// Returns results for each query in the same order
    pub async fn search_batch(&mut self, queries: &[&str], limit: usize) -> Result<Vec<Vec<SearchResult>>> {
        if queries.is_empty() {
            return Ok(vec![]);
        }

        // Check for empty queries
        for query in queries {
            if query.trim().is_empty() {
                return Err(SearchError::EmptyQuery);
            }
        }

        log::debug!("Batch hybrid search: {} queries, limit={}", queries.len(), limit);

        let candidate_pool = limit * 5;

        // Build chunk id -> index mapping (once for all queries)
        let mut chunk_id_to_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (idx, chunk) in self.chunks.iter().enumerate() {
            let id = format!("{}:{}:{}", chunk.file_path, chunk.start_line, chunk.end_line);
            chunk_id_to_idx.insert(id, idx);
        }

        // 1. Expand all queries
        let expanded_queries: Vec<String> = queries
            .iter()
            .map(|q| self.expander.expand_to_query(q))
            .collect();

        // 2. Batch semantic search with expanded queries
        let expanded_refs: Vec<&str> = expanded_queries.iter().map(|s| s.as_str()).collect();
        let semantic_results_batch = self.store.search_batch(&expanded_refs, candidate_pool).await?;
        log::debug!("Semantic batch: {} queries processed", semantic_results_batch.len());

        // 3. Process each query: fuzzy + RRF + AST boost
        let mut all_final_results = Vec::with_capacity(queries.len());
        for (i, query) in queries.iter().enumerate() {
            let semantic_results = &semantic_results_batch[i];

            // Convert semantic results to (chunk_idx, score)
            let semantic_scores: Vec<(usize, f32)> = semantic_results
                .iter()
                .filter_map(|result| {
                    chunk_id_to_idx
                        .get(&result.id)
                        .map(|&idx| (idx, result.score))
                })
                .collect();

            // Fuzzy search for this query
            let fuzzy_scores = self.fuzzy.search(query, &self.chunks, candidate_pool);

            // RRF Fusion with adaptive weights
            let fused_scores = self.fusion.fuse_adaptive(query, semantic_scores, fuzzy_scores);

            // AST-aware boosting
            let boosted_scores = AstBooster::boost(&self.chunks, fused_scores);

            // Convert to SearchResult
            let mut final_results: Vec<SearchResult> = boosted_scores
                .into_iter()
                .filter_map(|(idx, score)| {
                    self.chunks.get(idx).map(|chunk| {
                        let id = format!("{}:{}:{}", chunk.file_path, chunk.start_line, chunk.end_line);
                        SearchResult {
                            chunk: chunk.clone(),
                            score,
                            id,
                        }
                    })
                })
                .collect();

            // Sort and truncate
            final_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            final_results.truncate(limit);

            log::debug!("Query {}/{}: {} final results", i + 1, queries.len(), final_results.len());
            all_final_results.push(final_results);
        }

        log::info!("Batch hybrid search completed: {} queries", queries.len());
        Ok(all_final_results)
    }

    /// Semantic-only search (bypass fuzzy/fusion for speed)
    pub async fn search_semantic_only(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        if query.trim().is_empty() {
            return Err(SearchError::EmptyQuery);
        }

        self.store.search(query, limit).await.map_err(Into::into)
    }

    /// Get chunk by ID
    pub fn get_chunk(&self, id: &str) -> Option<&CodeChunk> {
        self.chunks.iter().find(|c| {
            let chunk_id = format!("{}:{}:{}", c.file_path, c.start_line, c.end_line);
            chunk_id == id
        })
    }

    /// Get all chunks
    pub fn chunks(&self) -> &[CodeChunk] {
        &self.chunks
    }
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
    #[ignore] // Requires FastEmbed model
    async fn test_hybrid_search() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("store.json");

        let chunks = vec![
            create_test_chunk("api.rs", 1, "handle_error", "async fn handle_error() { /* error handling */ }"),
            create_test_chunk("utils.rs", 20, "parse_data", "fn parse_data(input: &str) -> Result<Data> {}"),
            create_test_chunk("main.rs", 50, "main", "fn main() { println!(\"hello\"); }"),
        ];

        let mut store = VectorStore::new(&store_path).await.unwrap();
        store.add_chunks(chunks.clone()).await.unwrap();

        let mut search = HybridSearch::new(store, chunks).await.unwrap();

        let results = search.search("error handling", 5).await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    #[ignore] // Requires FastEmbed model
    async fn test_batch_search() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("store.json");

        let chunks = vec![
            create_test_chunk("api.rs", 1, "handle_error", "async fn handle_error() { /* error handling */ }"),
            create_test_chunk("utils.rs", 20, "parse_data", "fn parse_data(input: &str) -> Result<Data> {}"),
            create_test_chunk("main.rs", 50, "main", "fn main() { println!(\"hello\"); }"),
            create_test_chunk("db.rs", 100, "query_db", "async fn query_db(sql: &str) -> Result<Vec<Row>> {}"),
        ];

        let mut store = VectorStore::new(&store_path).await.unwrap();
        store.add_chunks(chunks.clone()).await.unwrap();

        let mut search = HybridSearch::new(store, chunks).await.unwrap();

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
        assert!(results_batch[0][0].chunk.content.contains("error")
                || results_batch[0][0].chunk.content.contains("handle_error"));
        assert!(results_batch[1][0].chunk.content.contains("parse")
                || results_batch[1][0].chunk.metadata.symbol_name.as_deref() == Some("parse_data"));
    }
}
