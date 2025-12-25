use context_indexer::ToolMeta;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExplainRequest {
    /// Symbol name to explain
    #[schemars(description = "Symbol name to get detailed information about")]
    pub symbol: String,

    /// Project directory path
    #[schemars(description = "Project directory path")]
    pub path: Option<String>,

    /// Programming language
    #[schemars(description = "Programming language: rust, python, javascript, typescript")]
    pub language: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ExplainResult {
    /// Symbol name
    pub symbol: String,
    /// Symbol kind (function, struct, etc.)
    pub kind: String,
    /// File path
    pub file: String,
    /// Line number
    pub line: usize,
    /// Documentation (if available)
    pub documentation: Option<String>,
    /// Dependencies (what this symbol uses/calls)
    pub dependencies: Vec<String>,
    /// Dependents (what uses/calls this symbol)
    pub dependents: Vec<String>,
    /// Related tests
    pub tests: Vec<String>,
    /// Code content
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ToolMeta>,
}
