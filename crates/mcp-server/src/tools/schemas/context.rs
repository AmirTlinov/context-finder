use context_indexer::ToolMeta;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ContextRequest {
    /// Search query
    #[schemars(description = "Natural language search query")]
    pub query: String,

    /// Project directory path
    #[schemars(description = "Project directory path")]
    pub path: Option<String>,

    /// Maximum primary results (default: 5)
    #[schemars(description = "Maximum number of primary results")]
    pub limit: Option<usize>,

    /// Search strategy: direct, extended, deep
    #[schemars(
        description = "Graph traversal depth: direct (none), extended (1-hop), deep (2-hop)"
    )]
    pub strategy: Option<String>,

    /// Graph language: rust, python, javascript, typescript
    #[schemars(description = "Programming language for graph analysis")]
    pub language: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ContextResult {
    /// Primary search results
    pub results: Vec<ContextHit>,
    /// Total related code found
    pub related_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ToolMeta>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ContextHit {
    /// File path
    pub file: String,
    /// Start line
    pub start_line: usize,
    /// End line
    pub end_line: usize,
    /// Symbol name
    pub symbol: Option<String>,
    /// Relevance score
    pub score: f32,
    /// Code content
    pub content: String,
    /// Related code through graph
    pub related: Vec<RelatedCode>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RelatedCode {
    /// File path
    pub file: String,
    /// Line range as string
    pub lines: String,
    /// Symbol name
    pub symbol: Option<String>,
    /// Relationship path (e.g., "Calls", "Uses -> Uses")
    pub relationship: String,
}
