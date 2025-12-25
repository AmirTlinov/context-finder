use context_code_chunker::{Chunker, ChunkerConfig, ChunkingStrategy, OverlapStrategy};

const RUST_CODE_WITH_SPLIT_IMPORTS: &str = r"use std::collections::HashMap;
use std::collections::HashSet;
// filler 1
// filler 2
// filler 3
// filler 4
// filler 5
// filler 6
// filler 7
// filler 8
pub fn foo() -> HashMap<i32, i32> {
    let _set: HashSet<i32> = HashSet::new();
    HashMap::new()
}
";

#[test]
fn contextual_infers_imports_for_line_chunking_without_mutating_content() {
    let config = ChunkerConfig {
        strategy: ChunkingStrategy::LineCount,
        overlap: OverlapStrategy::Contextual,
        // Keep the line-based strategy at 10 lines per chunk, but avoid the small-adjacent merge
        // (soft_threshold = target/2) by keeping the threshold below typical chunk sizes.
        target_chunk_tokens: 20,
        max_chunk_tokens: 10_000,
        min_chunk_tokens: 0,
        include_imports: true,
        include_parent_context: false,
        include_documentation: false,
        max_imports_per_chunk: 10,
        supported_languages: Vec::new(),
    };

    let chunks = Chunker::new(config)
        .chunk_str(RUST_CODE_WITH_SPLIT_IMPORTS, Some("sample.rs"))
        .expect("chunking rust");

    let foo = chunks
        .iter()
        .find(|chunk| chunk.content.contains("pub fn foo"))
        .expect("missing foo chunk");

    assert!(
        foo.metadata
            .context_imports
            .iter()
            .any(|imp| imp.contains("std::collections::HashMap")),
        "expected HashMap import in contextual imports"
    );
    assert!(
        foo.metadata
            .context_imports
            .iter()
            .any(|imp| imp.contains("std::collections::HashSet")),
        "expected HashSet import in contextual imports"
    );

    // Contextual overlap should not inject generated imports into the raw chunk content.
    assert!(!foo.content.contains("use std::collections::HashMap"));
    assert!(!foo.content.contains("use std::collections::HashSet"));
}
