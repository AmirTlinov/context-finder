use context_indexer::ToolMeta;
use serde::{Deserialize, Serialize};

pub const CONTEXT_PACK_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct ContextPackOutput {
    pub version: u32,
    pub query: String,
    pub model_id: String,
    pub profile: String,
    pub items: Vec<ContextPackItem>,
    pub budget: ContextPackBudget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ToolMeta>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextPackItem {
    pub id: String,
    pub role: String,
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub chunk_type: Option<String>,
    pub score: f32,
    pub imports: Vec<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextPackBudget {
    pub max_chars: usize,
    pub used_chars: usize,
    pub truncated: bool,
    pub dropped_items: usize,
}
