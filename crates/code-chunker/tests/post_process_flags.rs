use context_code_chunker::{ChunkType, Chunker, ChunkerConfig};

const RUST_CODE: &str = r"
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;

/// Hello docs
pub fn foo() -> HashMap<i32, i32> {
    let _set: HashSet<i32> = HashSet::new();
    let _dbg: &dyn Debug = &0;
    HashMap::new()
}

pub struct User;

impl User {
    pub fn method(&self) -> usize {
        1
    }
}
";

fn find_symbol<'a>(
    chunks: &'a [context_code_chunker::CodeChunk],
    name: &str,
) -> &'a context_code_chunker::CodeChunk {
    chunks
        .iter()
        .find(|chunk| chunk.metadata.symbol_name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing chunk for symbol {name}"))
}

#[test]
fn include_imports_false_clears_context_imports() {
    let config = ChunkerConfig {
        min_chunk_tokens: 0,
        include_imports: false,
        include_documentation: false,
        include_parent_context: false,
        ..Default::default()
    };
    let chunks = Chunker::new(config)
        .chunk_str(RUST_CODE, Some("sample.rs"))
        .expect("chunking rust");

    let foo = find_symbol(&chunks, "foo");
    assert!(foo.metadata.context_imports.is_empty());
}

#[test]
fn max_imports_per_chunk_limits_context_imports() {
    let config = ChunkerConfig {
        min_chunk_tokens: 0,
        include_imports: true,
        max_imports_per_chunk: 2,
        include_documentation: false,
        include_parent_context: false,
        ..Default::default()
    };
    let chunks = Chunker::new(config)
        .chunk_str(RUST_CODE, Some("sample.rs"))
        .expect("chunking rust");

    let foo = find_symbol(&chunks, "foo");
    assert!(foo.metadata.context_imports.len() <= 2);
}

#[test]
fn include_documentation_controls_metadata_and_keeps_content_raw() {
    let with_docs = ChunkerConfig {
        min_chunk_tokens: 0,
        include_imports: false,
        include_documentation: true,
        include_parent_context: false,
        ..Default::default()
    };
    let chunks = Chunker::new(with_docs)
        .chunk_str(RUST_CODE, Some("sample.rs"))
        .expect("chunking rust");
    let foo = find_symbol(&chunks, "foo");
    assert!(foo
        .metadata
        .documentation
        .as_deref()
        .unwrap_or_default()
        .contains("Hello docs"));
    assert!(!foo.content.contains("Hello docs"));

    let without_docs = ChunkerConfig {
        min_chunk_tokens: 0,
        include_imports: false,
        include_documentation: false,
        include_parent_context: false,
        ..Default::default()
    };
    let chunks = Chunker::new(without_docs)
        .chunk_str(RUST_CODE, Some("sample.rs"))
        .expect("chunking rust");
    let foo = find_symbol(&chunks, "foo");
    assert!(foo.metadata.documentation.is_none());
}

#[test]
fn include_parent_context_controls_parent_scope_and_qualified_name() {
    let with_parent = ChunkerConfig {
        min_chunk_tokens: 0,
        include_parent_context: true,
        include_imports: false,
        include_documentation: false,
        ..Default::default()
    };
    let chunks = Chunker::new(with_parent)
        .chunk_str(RUST_CODE, Some("sample.rs"))
        .expect("chunking rust");
    let method = find_symbol(&chunks, "method");
    assert_eq!(method.metadata.chunk_type, Some(ChunkType::Method));
    assert!(method
        .metadata
        .parent_scope
        .as_deref()
        .unwrap_or_default()
        .contains("User"));
    assert!(method
        .metadata
        .qualified_name
        .as_deref()
        .unwrap_or_default()
        .contains("User"));

    let without_parent = ChunkerConfig {
        min_chunk_tokens: 0,
        include_parent_context: false,
        include_imports: false,
        include_documentation: false,
        ..Default::default()
    };
    let chunks = Chunker::new(without_parent)
        .chunk_str(RUST_CODE, Some("sample.rs"))
        .expect("chunking rust");
    let method = find_symbol(&chunks, "method");
    assert!(method.metadata.parent_scope.is_none());
    assert_eq!(method.metadata.qualified_name.as_deref(), Some("method"));
}
