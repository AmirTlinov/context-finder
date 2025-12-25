use context_indexer::ToolMeta;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TraceRequest {
    /// Start symbol
    #[schemars(description = "Starting symbol name")]
    pub from: String,

    /// End symbol
    #[schemars(description = "Target symbol name")]
    pub to: String,

    /// Project directory path
    #[schemars(description = "Project directory path")]
    pub path: Option<String>,

    /// Programming language
    #[schemars(description = "Programming language: rust, python, javascript, typescript")]
    pub language: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TraceResult {
    /// Whether path was found
    pub found: bool,
    /// Call chain path
    pub path: Vec<TraceStep>,
    /// Path depth
    pub depth: usize,
    /// Mermaid sequence diagram
    pub mermaid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ToolMeta>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct TraceStep {
    /// Symbol name
    pub symbol: String,
    /// File path
    pub file: String,
    /// Line number
    pub line: usize,
    /// Relationship to next step
    pub relationship: Option<String>,
}
