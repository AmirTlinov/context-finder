use context_indexer::ToolMeta;
use context_protocol::Capabilities;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CapabilitiesRequest {
    /// Project directory path (optional, used for meta.index_state)
    #[schemars(
        description = "Project directory path (defaults to session root; fallback: CONTEXT_FINDER_ROOT/CONTEXT_FINDER_PROJECT_ROOT, git root, then cwd)."
    )]
    pub path: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CapabilitiesResult {
    #[serde(flatten)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub meta: ToolMeta,
}
