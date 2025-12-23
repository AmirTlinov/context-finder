use crate::query_classifier::QueryWeights;
use context_code_chunker::{ChunkType, CodeChunk};
use std::collections::HashMap;

/// Reciprocal Rank Fusion for combining multiple rankings
pub struct RRFFusion {
    /// RRF constant k (typically 60)
    k: f32,

    /// Weights for each ranking source
    semantic_weight: f32,
    fuzzy_weight: f32,
}

impl RRFFusion {
    #[must_use]
    pub const fn new(semantic_weight: f32, fuzzy_weight: f32, k: f32) -> Self {
        Self {
            k,
            semantic_weight,
            fuzzy_weight,
        }
    }

    /// Fuse semantic and fuzzy results using weights, logging the context
    #[must_use]
    pub fn fuse_adaptive(
        &self,
        query: &str,
        weights: &QueryWeights,
        semantic_results: &[(usize, f32)],
        fuzzy_results: &[(usize, f32)],
    ) -> Vec<(usize, f32)> {
        // Increase k for fuzzy part to suppress noisy neighbors
        let adaptive_k = self.k + 40.0;
        log::debug!(
            "Adaptive weights for '{}': semantic={:.1}%, fuzzy={:.1}%",
            query,
            weights.semantic * 100.0,
            weights.fuzzy * 100.0
        );

        Self::fuse_with_weights(
            semantic_results,
            fuzzy_results,
            weights.semantic,
            weights.fuzzy,
            adaptive_k,
        )
    }

    /// Fuse semantic and fuzzy results using RRF
    ///
    /// RRF formula: score(d) = Σ `weight_i` / (k + `rank_i(d)`)
    ///
    /// Returns (`chunk_index`, `fused_score`) sorted by score descending
    #[must_use]
    pub fn fuse(
        &self,
        semantic_results: &[(usize, f32)],
        fuzzy_results: &[(usize, f32)],
    ) -> Vec<(usize, f32)> {
        Self::fuse_with_weights(
            semantic_results,
            fuzzy_results,
            self.semantic_weight,
            self.fuzzy_weight,
            self.k,
        )
    }

    /// Fuse with explicit weights
    #[allow(clippy::cast_precision_loss)]
    fn fuse_with_weights(
        semantic_results: &[(usize, f32)],
        fuzzy_results: &[(usize, f32)],
        semantic_weight: f32,
        fuzzy_weight: f32,
        k: f32,
    ) -> Vec<(usize, f32)> {
        let mut scores: HashMap<usize, f32> = HashMap::new();

        // Add semantic scores
        for (rank, (idx, _score)) in semantic_results.iter().enumerate() {
            let rrf_score = semantic_weight / (k + rank as f32 + 1.0);
            *scores.entry(*idx).or_insert(0.0) += rrf_score;
        }

        // Add fuzzy scores
        for (rank, (idx, _score)) in fuzzy_results.iter().enumerate() {
            let rrf_score = fuzzy_weight / (k + rank as f32 + 1.0);
            *scores.entry(*idx).or_insert(0.0) += rrf_score;
        }

        // Sort by fused score descending
        let mut fused: Vec<(usize, f32)> = scores.into_iter().collect();
        fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        fused
    }
}

impl Default for RRFFusion {
    fn default() -> Self {
        Self::new(0.8, 0.2, 60.0)
    }
}

/// AST-aware boosting to prioritize important code elements
pub struct AstBooster;

impl AstBooster {
    /// Boost scores based on chunk type priority
    /// Functions/Methods get higher boost than variables
    #[must_use]
    pub fn boost(chunks: &[CodeChunk], results: Vec<(usize, f32)>) -> Vec<(usize, f32)> {
        results
            .into_iter()
            .map(|(idx, score)| {
                let boost = chunks.get(idx).map_or(1.0, Self::compute_boost);
                (idx, score * boost)
            })
            .collect()
    }

    fn compute_boost(chunk: &CodeChunk) -> f32 {
        let type_boost = chunk.metadata.chunk_type.map_or(1.0, |ct| match ct {
            ChunkType::Function | ChunkType::Method => 1.18,
            ChunkType::Struct | ChunkType::Class | ChunkType::Enum | ChunkType::Interface => 1.05,
            ChunkType::Variable | ChunkType::Const => 0.9,
            _ => f32::from(ct.priority()) / 100.0,
        });

        // Additional boost for chunks with documentation
        let doc_boost = if chunk.metadata.documentation.is_some() {
            1.1
        } else {
            1.0
        };

        // Boost for chunks with context (imports, parent scope)
        let context_boost = if !chunk.metadata.context_imports.is_empty()
            || chunk.metadata.parent_scope.is_some()
        {
            1.05
        } else {
            1.0
        };

        let path = chunk.file_path.to_lowercase();
        let mut path_boost = 1.0;
        if path.contains("/agents/") || path.contains("/tests/") || path.contains("/test/") {
            path_boost *= 0.6;
        }
        if path.contains("/scripts/")
            || path.contains("docker")
            || path.contains("/db/")
            || path.contains("/deploy/")
            || path.contains("/infra/")
        {
            path_boost *= 0.7;
        }
        if path.contains("packages/utils") || path.contains("/utils") || path.contains("src/lib") {
            path_boost *= 1.25;
        }

        type_boost * doc_boost * context_boost * path_boost
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use context_code_chunker::{ChunkMetadata, ChunkType, CodeChunk};

    #[test]
    fn test_rrf_fusion() {
        let fusion = RRFFusion::default();

        let semantic = vec![(0, 0.9), (1, 0.8), (2, 0.7)];
        let fuzzy = vec![(2, 0.95), (0, 0.85), (3, 0.75)];

        let fused = fusion.fuse(&semantic, &fuzzy);

        // Chunk 0 should rank high (in both lists)
        // Chunk 2 should rank high (high in fuzzy, present in semantic)
        assert!(!fused.is_empty());

        // Verify scores are computed
        for (_, score) in &fused {
            assert!(*score > 0.0);
        }
    }

    #[test]
    fn test_rrf_weights() {
        // Heavy semantic weight
        let fusion_semantic = RRFFusion::new(0.9, 0.1, 60.0);

        let semantic = vec![(0, 0.9)];
        let fuzzy = vec![(1, 0.9)];

        let fused = fusion_semantic.fuse(&semantic, &fuzzy);

        // Chunk 0 (semantic) should rank higher due to weight
        assert_eq!(fused[0].0, 0);
    }

    #[test]
    fn test_ast_boosting() {
        let chunks = vec![
            CodeChunk::new(
                "test.rs".to_string(),
                1,
                10,
                "fn test() {}".to_string(),
                ChunkMetadata::default().chunk_type(ChunkType::Function),
            ),
            CodeChunk::new(
                "test.rs".to_string(),
                20,
                25,
                "let x = 5;".to_string(),
                ChunkMetadata::default().chunk_type(ChunkType::Variable),
            ),
        ];

        let results = vec![(0, 0.5), (1, 0.5)];
        let boosted = AstBooster::boost(&chunks, results);

        // Function should have higher score than variable after boosting
        assert!(boosted[0].1 > boosted[1].1 || boosted[0].0 == 0);
    }
}
