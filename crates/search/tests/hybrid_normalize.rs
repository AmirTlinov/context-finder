use context_code_chunker::{ChunkMetadata, CodeChunk};
use context_search::hybrid::HybridSearch;
use context_vector_store::SearchResult;

fn make_result(id: &str, score: f32) -> SearchResult {
    SearchResult {
        id: id.to_string(),
        score,
        chunk: CodeChunk::new(
            "file.rs".into(),
            1,
            1,
            "fn demo() {}".into(),
            ChunkMetadata {
                language: Some("rust".into()),
                ..ChunkMetadata::default()
            },
        ),
    }
}

#[test]
fn normalize_scores_skips_non_finite_and_handles_singleton() {
    let mut results = vec![make_result("a", f32::NAN), make_result("b", 10.0)];

    HybridSearch::normalize_scores(&mut results);

    assert_eq!(results[0].score, 0.0, "NaN must be reset to 0");
    assert_eq!(results[1].score, 1.0, "Max score should normalize to 1");
}

#[test]
fn normalize_scores_avoids_tiny_delta_and_inf() {
    let mut results = vec![
        make_result("a", 1.0),
        make_result("b", 1.0 + 5e-7), // below MIN_DELTA
        make_result("c", f32::INFINITY),
    ];

    HybridSearch::normalize_scores(&mut results);

    for res in &results {
        assert!(
            (res.score - 1.0).abs() < f32::EPSILON,
            "All scores should be equal when delta is tiny"
        );
    }
}
