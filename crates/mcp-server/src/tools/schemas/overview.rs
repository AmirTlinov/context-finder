use context_indexer::ToolMeta;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OverviewRequest {
    /// Project directory path
    #[schemars(description = "Project directory path")]
    pub path: Option<String>,

    /// Programming language
    #[schemars(description = "Programming language: rust, python, javascript, typescript")]
    pub language: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct OverviewResult {
    /// Project info
    pub project: ProjectInfo,
    /// Architecture layers
    pub layers: Vec<LayerInfo>,
    /// Entry points
    pub entry_points: Vec<String>,
    /// Key types (most connected)
    pub key_types: Vec<KeyTypeInfo>,
    /// Graph statistics
    pub graph_stats: GraphStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ToolMeta>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProjectInfo {
    pub name: String,
    pub files: usize,
    pub chunks: usize,
    pub lines: usize,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct LayerInfo {
    pub name: String,
    pub files: usize,
    pub role: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct KeyTypeInfo {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub coupling: usize,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GraphStats {
    pub nodes: usize,
    pub edges: usize,
}
