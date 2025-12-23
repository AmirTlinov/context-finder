use crate::{ContextPackBudget, ContextPackItem};
use serde::{Deserialize, Serialize};

pub const TASK_PACK_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TaskPackItem {
    #[serde(flatten)]
    pub item: ContextPackItem,
    #[serde(default)]
    pub why: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum NextActionKind {
    OpenFile,
    Run,
    Query,
    UpdateContract,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NextAction {
    pub kind: NextActionKind,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TaskPackOutput {
    pub version: u32,
    pub intent: String,
    pub model_id: String,
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    pub items: Vec<TaskPackItem>,
    pub next_actions: Vec<NextAction>,
    pub budget: ContextPackBudget,
}
