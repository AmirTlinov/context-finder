use rmcp::schemars;
use serde::Deserialize;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ContextPackRequest {
    /// Search query
    #[schemars(description = "Natural language search query")]
    pub query: String,

    /// Project directory path
    #[schemars(description = "Project directory path")]
    pub path: Option<String>,

    /// Maximum primary results (default: 10)
    #[schemars(description = "Maximum number of primary results")]
    pub limit: Option<usize>,

    /// Maximum total characters for packed output (default: 20000)
    #[schemars(description = "Maximum total characters in packed output")]
    pub max_chars: Option<usize>,

    /// Related chunks per primary (default: 3)
    #[schemars(description = "Maximum related chunks per primary")]
    pub max_related_per_primary: Option<usize>,

    /// Prefer code results over markdown docs (implementation-first).
    #[schemars(description = "Prefer code results over markdown docs (implementation-first)")]
    pub prefer_code: Option<bool>,

    /// Whether markdown docs (e.g. *.md) may be included in the pack (default: true).
    #[schemars(description = "Whether markdown docs (e.g. *.md) may be included in the pack")]
    pub include_docs: Option<bool>,

    /// Related context mode: "explore" (default) or "focus" (query-gated).
    #[schemars(description = "Related context mode: 'explore' (default) or 'focus' (query-gated)")]
    pub related_mode: Option<String>,

    /// Search strategy: direct, extended, deep
    #[schemars(
        description = "Graph traversal depth: direct (none), extended (1-hop), deep (2-hop)"
    )]
    pub strategy: Option<String>,

    /// Graph language: rust, python, javascript, typescript
    #[schemars(description = "Programming language for graph analysis")]
    pub language: Option<String>,

    /// Auto-index the project if missing (opt-in)
    #[schemars(
        description = "If true, automatically index the project (single-model) when missing before building the context pack"
    )]
    pub auto_index: Option<bool>,

    /// Include debug output (adds a second MCP content block with debug JSON)
    #[schemars(description = "Include debug output as an additional response block")]
    pub trace: Option<bool>,
}
