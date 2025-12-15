mod context;
pub mod domain;
pub mod infra;
mod services;
pub mod warm;

#[allow(unused_imports)]
pub use domain::{
    classify_error, CommandAction, CommandRequest, CommandResponse, CommandStatus,
    ContextPackOutput, ContextPackPayload, EvalCacheMode, EvalCaseResult, EvalCompareCase,
    EvalCompareConfig, EvalCompareOutput, EvalComparePayload, EvalCompareSummary, EvalDatasetMeta,
    EvalHit, EvalOutput, EvalPayload, EvalRun, EvalRunSummary, EvalSummary, Hint, HintKind,
    IndexPayload, IndexResponse, ListSymbolsPayload, MapOutput, MapPayload, ResponseMeta,
    SearchOutput, SearchPayload, SearchStrategy, SearchWithContextPayload, SymbolsOutput,
};

use crate::cache::CacheConfig;
use anyhow::Result;
use domain::CommandOutcome;
use services::Services;

pub struct CommandHandler {
    services: Services,
}

impl CommandHandler {
    pub fn new(cache_cfg: CacheConfig) -> Self {
        Self {
            services: Services::new(cache_cfg),
        }
    }

    pub async fn execute(&self, request: CommandRequest) -> Result<CommandResponse> {
        let CommandRequest {
            action,
            payload,
            config,
        } = request;

        let mut outcome: CommandOutcome = self
            .services
            .route(action, payload, context::CommandContext::new(config))
            .await?;

        outcome.meta.duration_ms = outcome
            .meta
            .duration_ms
            .or_else(|| Some(outcome.started.elapsed().as_millis() as u64));

        Ok(CommandResponse {
            status: CommandStatus::Ok,
            message: None,
            hints: outcome.hints,
            data: outcome.data,
            meta: outcome.meta,
        })
    }
}

pub async fn execute(request: CommandRequest, cache_cfg: CacheConfig) -> Result<CommandResponse> {
    CommandHandler::new(cache_cfg).execute(request).await
}
