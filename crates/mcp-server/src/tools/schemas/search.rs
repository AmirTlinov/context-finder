use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchRequest {
    /// Search query (semantic search)
    #[schemars(description = "Natural language search query")]
    pub query: String,

    /// Project directory path
    #[schemars(description = "Project directory path")]
    pub path: Option<String>,

    /// Maximum results (default: 10)
    #[schemars(description = "Maximum number of results (1-50)")]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchResult {
    /// File path
    pub file: String,
    /// Start line
    pub start_line: usize,
    /// End line
    pub end_line: usize,
    /// Symbol name (if any)
    pub symbol: Option<String>,
    /// Symbol type (function, struct, etc.)
    pub symbol_type: Option<String>,
    /// Relevance score (0-1)
    pub score: f32,
    /// Code content
    pub content: String,
}
