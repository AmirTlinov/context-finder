pub mod batch;
pub mod context;
pub mod context_pack;
pub mod doctor;
pub mod explain;
pub mod file_slice;
pub mod grep_context;
pub mod impact;
pub mod index;
pub mod list_files;
pub mod map;
pub mod overview;
pub mod read_pack;
pub mod repo_onboarding_pack;
pub mod search;
pub mod text_search;
pub mod trace;

use rmcp::schemars;
use serde::Serialize;

#[derive(Debug, Serialize, schemars::JsonSchema, Clone)]
pub struct ToolNextAction {
    pub tool: String,
    pub args: serde_json::Value,
    pub reason: String,
}
