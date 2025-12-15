use context_code_chunker::{ChunkType, Chunker, ChunkerConfig, CodeChunk};

const RUST_IMPLS: &str = r#"
mod inner {
    pub struct Wrapper<'a> {
        pub items: &'a [u8],
    }

    pub struct User;
}

use std::fmt;

impl fmt::Display for inner::User {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = "format user";
        write!(f, "{msg}")
    }
}

impl<'a> inner::Wrapper<'a> {
    pub fn len(&self) -> usize {
        let slice = self.items;
        slice.len()
    }
}

impl<'a> AsRef<[u8]> for &'a [u8] {
    fn as_ref(&self) -> &[u8] {
        let slice = *self;
        slice
    }
}
"#;

fn rust_chunks(code: &str) -> Vec<CodeChunk> {
    let chunker = Chunker::new(ChunkerConfig::default());
    chunker
        .chunk_str(code, Some("sample.rs"))
        .expect("chunking Rust impls")
}

fn method_parent_scope(chunks: &[CodeChunk], name: &str) -> String {
    let chunk = chunks
        .iter()
        .find(|chunk| {
            chunk.metadata.chunk_type == Some(ChunkType::Method)
                && chunk.metadata.symbol_name.as_deref() == Some(name)
        })
        .unwrap_or_else(|| panic!("missing method chunk for {name}"));

    chunk
        .metadata
        .parent_scope
        .clone()
        .unwrap_or_else(|| panic!("missing parent_scope for {name}"))
}

#[test]
fn ast_analyzer_trait_impl_uses_type_after_for_keyword() {
    let chunks = rust_chunks(RUST_IMPLS);
    let fmt_scope = method_parent_scope(&chunks, "fmt");

    assert_eq!(fmt_scope, "inner::User");
}

#[test]
fn ast_analyzer_preserves_scoped_and_reference_targets() {
    let chunks = rust_chunks(RUST_IMPLS);

    let len_scope = method_parent_scope(&chunks, "len");
    let as_ref_scope = method_parent_scope(&chunks, "as_ref");

    assert_eq!(len_scope, "inner::Wrapper<'a>");
    assert_eq!(as_ref_scope, "&'a [u8]");
}
