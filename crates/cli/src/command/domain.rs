use anyhow::Result;
pub use context_search::{ContextPackBudget, ContextPackItem, ContextPackOutput};
pub use context_search::{
    NextAction, NextActionKind, TaskPackItem, TaskPackOutput, TASK_PACK_VERSION,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::PathBuf;

pub const DEFAULT_LIMIT: usize = 10;
pub const DEFAULT_CONTEXT_WINDOW: usize = 20;
pub const BATCH_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
pub struct CommandRequest {
    pub action: CommandAction,
    #[serde(default = "empty_payload")]
    pub payload: Value,
    #[serde(default)]
    pub options: Option<RequestOptions>,
    #[serde(default)]
    pub config: Option<Value>,
}

fn empty_payload() -> Value {
    Value::Object(Default::default())
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandAction {
    Search,
    SearchWithContext,
    ContextPack,
    TaskPack,
    TextSearch,
    Batch,
    Index,
    GetContext,
    ListSymbols,
    ConfigRead,
    CompareSearch,
    Map,
    Eval,
    EvalCompare,
}

#[derive(Debug, Deserialize)]
pub struct BatchPayload {
    #[serde(default)]
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub max_chars: Option<usize>,
    #[serde(default)]
    pub stop_on_error: bool,
    pub items: Vec<BatchItem>,
}

#[derive(Debug, Deserialize)]
pub struct BatchItem {
    pub id: String,
    pub action: CommandAction,
    #[serde(default = "empty_payload")]
    pub payload: Value,
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct BatchBudget {
    pub max_chars: usize,
    pub used_chars: usize,
    pub truncated: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct BatchItemResult {
    pub id: String,
    pub status: CommandStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<Hint>,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub meta: ResponseMeta,
}

#[derive(Debug, Serialize, Clone)]
pub struct BatchOutput {
    pub version: u32,
    pub items: Vec<BatchItemResult>,
    pub budget: BatchBudget,
}

#[derive(Debug, Serialize)]
pub struct CommandResponse {
    pub status: CommandStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<Hint>,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub meta: ResponseMeta,
}

impl CommandResponse {
    pub fn is_error(&self) -> bool {
        matches!(self.status, CommandStatus::Error)
    }
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Ok,
    Error,
}

#[derive(Debug, Serialize, Clone)]
pub struct Hint {
    #[serde(rename = "type")]
    pub kind: HintKind,
    pub text: String,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum HintKind {
    Info,
    Cache,
    Action,
    Warn,
    Deprecation,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RequestOptions {
    #[serde(default)]
    pub stale_policy: StalePolicy,
    #[serde(default = "default_max_reindex_ms")]
    pub max_reindex_ms: u64,
    #[serde(default = "default_true")]
    pub allow_filesystem_fallback: bool,
    #[serde(default)]
    pub include_paths: Vec<String>,
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    #[serde(default)]
    pub file_pattern: Option<String>,
}

impl Default for RequestOptions {
    fn default() -> Self {
        Self {
            stale_policy: StalePolicy::default(),
            max_reindex_ms: default_max_reindex_ms(),
            allow_filesystem_fallback: default_true(),
            include_paths: Vec::new(),
            exclude_paths: Vec::new(),
            file_pattern: None,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_max_reindex_ms() -> u64 {
    3000
}

#[derive(Debug, Deserialize, Copy, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StalePolicy {
    #[default]
    Auto,
    Warn,
    Fail,
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct ResponseMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_updated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_mtime_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_nodes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_edges: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_files: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_chunks: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_cache_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_last_success_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_last_failure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warm: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warm_cost_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warm_graph_cache_hit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicates_dropped: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_spans_dropped: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_load_index_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_graph_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_search_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_last_failure_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_failure_reasons: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_p95_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_failure_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_files_per_sec: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_stale_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_pending_events: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_state: Option<context_indexer::IndexState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_avg_baseline_ms: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_avg_context_ms: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_avg_overlap_ratio: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_avg_related: Option<f32>,
}

pub struct CommandOutcome {
    pub data: Value,
    pub hints: Vec<Hint>,
    pub meta: ResponseMeta,
}

impl CommandOutcome {
    pub fn from_value<T: Serialize>(value: T) -> Result<Self> {
        Ok(Self {
            data: serde_json::to_value(value)?,
            hints: Vec::new(),
            meta: ResponseMeta::default(),
        })
    }
}

pub fn parse_payload<T: DeserializeOwned>(payload: Value) -> Result<T> {
    serde_json::from_value(payload).map_err(Into::into)
}

pub fn merge_configs(base: Option<Value>, overrides: Option<Value>) -> Option<Value> {
    match (base, overrides) {
        (None, None) => None,
        (Some(mut base_value), Some(override_value)) => {
            merge_json(&mut base_value, &override_value);
            Some(base_value)
        }
        (Some(base_value), None) => Some(base_value),
        (None, Some(override_value)) => Some(override_value),
    }
}

fn merge_json(base: &mut Value, overlay: &Value) {
    if let Value::Object(overlay_map) = overlay {
        if !base.is_object() {
            *base = Value::Object(Map::new());
        }

        if let Value::Object(base_map) = base {
            for (key, value) in overlay_map {
                match base_map.get_mut(key) {
                    Some(existing) => merge_json(existing, value),
                    None => {
                        base_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    } else {
        *base = overlay.clone();
    }
}

fn config_lookup<'a>(config: &'a Option<Value>, path: &[&str]) -> Option<&'a Value> {
    let mut current = config.as_ref()?;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

pub fn config_string_path(config: &Option<Value>, path: &[&str]) -> Option<String> {
    config_lookup(config, path)
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

pub fn config_bool_path(config: &Option<Value>, path: &[&str]) -> Option<bool> {
    config_lookup(config, path).and_then(Value::as_bool)
}

pub fn config_usize_path(config: &Option<Value>, path: &[&str]) -> Option<usize> {
    config_lookup(config, path)
        .and_then(Value::as_u64)
        .map(|raw| raw as usize)
}

pub fn normalize_config(config: Option<Value>) -> Option<Value> {
    config.and_then(|value| if value.is_null() { None } else { Some(value) })
}

#[derive(Debug, Deserialize, Serialize)]
pub struct IndexPayload {
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub full: bool,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub experts: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EvalPayload {
    #[serde(default)]
    pub path: Option<PathBuf>,
    pub dataset: PathBuf,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub cache_mode: Option<EvalCacheMode>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum EvalCacheMode {
    Warm,
    Cold,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalOutput {
    pub dataset: EvalDatasetMeta,
    pub runs: Vec<EvalRun>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalDatasetMeta {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub cases: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalRun {
    pub profile: String,
    pub models: Vec<String>,
    pub limit: usize,
    pub cache_mode: EvalCacheMode,
    pub summary: EvalSummary,
    pub cases: Vec<EvalCaseResult>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalRunSummary {
    pub profile: String,
    pub models: Vec<String>,
    pub limit: usize,
    pub cache_mode: EvalCacheMode,
    pub summary: EvalSummary,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalSummary {
    pub mean_mrr: f64,
    pub mean_recall: f64,
    pub mean_overlap_ratio: f64,
    pub mean_latency_ms: f64,
    pub p50_latency_ms: u64,
    pub p95_latency_ms: u64,
    pub mean_bytes: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalCaseResult {
    pub id: String,
    pub query: String,
    pub expected_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_symbols: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    pub mrr: f64,
    pub recall: f64,
    pub overlap_ratio: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_rank: Option<usize>,
    pub latency_ms: u64,
    pub bytes: usize,
    pub hits: Vec<EvalHit>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalHit {
    pub id: String,
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub score: f32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EvalComparePayload {
    #[serde(default)]
    pub path: Option<PathBuf>,
    pub dataset: PathBuf,
    #[serde(default)]
    pub limit: Option<usize>,
    pub a: EvalCompareConfig,
    pub b: EvalCompareConfig,
    #[serde(default)]
    pub cache_mode: Option<EvalCacheMode>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EvalCompareConfig {
    pub profile: String,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalCompareOutput {
    pub dataset: EvalDatasetMeta,
    pub cache_mode: EvalCacheMode,
    pub a: EvalRunSummary,
    pub b: EvalRunSummary,
    pub summary: EvalCompareSummary,
    pub cases: Vec<EvalCompareCase>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalCompareSummary {
    pub delta_mean_mrr: f64,
    pub delta_mean_recall: f64,
    pub delta_mean_overlap_ratio: f64,
    pub delta_mean_latency_ms: f64,
    pub delta_p95_latency_ms: i64,
    pub delta_mean_bytes: f64,
    pub a_wins: usize,
    pub b_wins: usize,
    pub ties: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalCompareCase {
    pub id: String,
    pub query: String,
    pub expected_paths: Vec<String>,
    pub a_mrr: f64,
    pub b_mrr: f64,
    pub delta_mrr: f64,
    pub a_recall: f64,
    pub b_recall: f64,
    pub delta_recall: f64,
    pub a_overlap_ratio: f64,
    pub b_overlap_ratio: f64,
    pub delta_overlap_ratio: f64,
    pub a_latency_ms: u64,
    pub b_latency_ms: u64,
    pub delta_latency_ms: i64,
    pub a_bytes: usize,
    pub b_bytes: usize,
    pub delta_bytes: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a_first_rank: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b_first_rank: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SearchPayload {
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub trace: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchWithContextPayload {
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub strategy: Option<SearchStrategy>,
    #[serde(default)]
    pub show_graph: Option<bool>,
    #[serde(default)]
    pub trace: Option<bool>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub reuse_graph: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TextSearchPayload {
    pub pattern: String,
    #[serde(default)]
    pub max_results: Option<usize>,
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    #[serde(default)]
    pub whole_word: Option<bool>,
    #[serde(default)]
    pub project: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TextSearchMatch {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TextSearchOutput {
    pub pattern: String,
    pub source: String,
    pub scanned_files: usize,
    pub matched_files: usize,
    pub skipped_large_files: usize,
    pub returned: usize,
    pub truncated: bool,
    pub matches: Vec<TextSearchMatch>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContextPackPayload {
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub strategy: Option<SearchStrategy>,
    #[serde(default)]
    pub max_chars: Option<usize>,
    #[serde(default)]
    pub max_related_per_primary: Option<usize>,
    #[serde(default)]
    pub trace: Option<bool>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub reuse_graph: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskPackPayload {
    pub intent: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub strategy: Option<SearchStrategy>,
    #[serde(default)]
    pub max_chars: Option<usize>,
    #[serde(default)]
    pub max_related_per_primary: Option<usize>,
    #[serde(default)]
    pub trace: Option<bool>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub reuse_graph: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CompareSearchPayload {
    #[serde(default)]
    pub queries: Vec<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub strategy: Option<SearchStrategy>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub reuse_graph: Option<bool>,
    #[serde(default)]
    pub show_graph: Option<bool>,
    #[serde(default)]
    pub invalidate_cache: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SearchStrategy {
    Direct,
    #[default]
    Extended,
    Deep,
}

impl SearchStrategy {
    pub const fn to_assembly(self) -> context_graph::AssemblyStrategy {
        match self {
            SearchStrategy::Direct => context_graph::AssemblyStrategy::Direct,
            SearchStrategy::Extended => context_graph::AssemblyStrategy::Extended,
            SearchStrategy::Deep => context_graph::AssemblyStrategy::Deep,
        }
    }

    pub fn from_name(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            "direct" => Some(SearchStrategy::Direct),
            "extended" => Some(SearchStrategy::Extended),
            "deep" => Some(SearchStrategy::Deep),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            SearchStrategy::Direct => "direct",
            SearchStrategy::Extended => "extended",
            SearchStrategy::Deep => "deep",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GetContextPayload {
    pub file: String,
    pub line: usize,
    #[serde(default = "default_window")]
    pub window: usize,
    #[serde(default)]
    pub project: Option<PathBuf>,
}

fn default_window() -> usize {
    DEFAULT_CONTEXT_WINDOW
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ListSymbolsPayload {
    pub file: String,
    #[serde(default)]
    pub project: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ConfigReadPayload {
    #[serde(default)]
    pub project: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
pub struct IndexResponse {
    pub stats: context_indexer::IndexStats,
}

#[derive(Serialize)]
pub struct ConfigReadResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

#[derive(Serialize, Deserialize)]
pub struct SearchOutput {
    pub query: String,
    pub results: Vec<SearchResultOutput>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ComparisonOutput {
    pub project: String,
    pub limit: usize,
    pub strategy: String,
    pub reuse_graph: bool,
    pub queries: Vec<QueryComparison>,
    pub summary: ComparisonSummary,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct QueryComparison {
    pub query: String,
    pub limit: usize,
    pub baseline_duration_ms: u64,
    pub context_duration_ms: u64,
    pub overlap: usize,
    pub overlap_ratio: f32,
    pub context_related: usize,
    pub baseline: Vec<SearchResultOutput>,
    pub context: Vec<SearchResultOutput>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ComparisonSummary {
    pub avg_baseline_ms: f32,
    pub avg_context_ms: f32,
    pub avg_overlap_ratio: f32,
    pub avg_related_chunks: f32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SearchResultOutput {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub symbol: Option<String>,
    #[serde(rename = "type")]
    pub chunk_type: Option<String>,
    pub score: f32,
    pub content: String,
    pub context: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Vec<RelatedCodeOutput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<Vec<RelationshipOutput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RelatedCodeOutput {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub symbol: Option<String>,
    pub relationship: Vec<String>,
    pub distance: usize,
    pub relevance: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RelationshipOutput {
    pub from: String,
    pub to: String,
    pub relationship: String,
}

#[derive(Serialize)]
pub struct ContextOutput {
    pub file: String,
    pub line: usize,
    pub symbol: Option<String>,
    #[serde(rename = "type")]
    pub chunk_type: Option<String>,
    pub parent: Option<String>,
    pub imports: Vec<String>,
    pub content: String,
    pub window: WindowOutput,
}

#[derive(Serialize)]
pub struct WindowOutput {
    pub before: String,
    pub after: String,
}

#[derive(Serialize, Deserialize)]
pub struct SymbolsOutput {
    /// File name (for single-file mode) or pattern used
    pub file: String,
    /// All symbols found
    pub symbols: Vec<SymbolInfo>,
    /// Number of files processed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_count: Option<usize>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SymbolInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub symbol_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub line: usize,
    /// File path (for multi-file mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MapPayload {
    #[serde(default)]
    pub project: Option<PathBuf>,
    #[serde(default = "map_default_depth")]
    pub depth: usize,
    #[serde(default)]
    pub limit: Option<usize>,
}

fn map_default_depth() -> usize {
    2
}

#[derive(Serialize, Deserialize)]
pub struct MapOutput {
    pub nodes: Vec<MapNode>,
    pub total_files: usize,
    pub total_chunks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_chunks_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_files_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_lines_pct: Option<f32>,
}

#[derive(Serialize, Deserialize)]
pub struct MapNode {
    pub path: String,
    pub files: usize,
    pub chunks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_chunks_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_files_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_lines_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_symbols: Option<Vec<SymbolInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_symbol_coverage: Option<f32>,
}

pub fn classify_error(message: &str) -> Vec<Hint> {
    let mut hints = Vec::new();

    if message.contains("Index not found") {
        hints.push(Hint {
            kind: HintKind::Action,
            text: "Index missing — run Command {\"action\":\"index\",\"payload\":{\"path\":\".\"}} to build it."
                .to_string(),
        });
    }

    if message.contains("Failed to load vector store") {
        hints.push(Hint {
            kind: HintKind::Action,
            text: "Index file looks corrupted — delete .context-finder/indexes/<model_id>/index.json and rerun the index action.".to_string(),
        });
    }

    if message.contains("Failed to read metadata") {
        hints.push(Hint {
            kind: HintKind::Warn,
            text: "Filesystem metadata unavailable — check permissions or run from inside the project directory.".to_string(),
        });
    }

    if message.to_lowercase().contains("graph language") {
        hints.push(Hint {
            kind: HintKind::Action,
            text: "Specify graph_language in config or payload to enable context graph assembly."
                .to_string(),
        });
    }

    if message.to_lowercase().contains("config") {
        hints.push(Hint {
            kind: HintKind::Warn,
            text: "Config issue detected — verify .context-finder/config.json or remove it."
                .to_string(),
        });
    }

    if message.contains("Project path does not exist") {
        hints.push(Hint {
            kind: HintKind::Action,
            text: "Check payload.path/project or run from the repository root.".to_string(),
        });
    }

    if message.contains("File not found") {
        hints.push(Hint {
            kind: HintKind::Action,
            text: "Verify the 'file' value is relative to project root and exists on disk."
                .to_string(),
        });
    }

    hints
}
