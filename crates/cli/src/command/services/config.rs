use crate::command::context::CommandContext;
use crate::command::domain::{
    parse_payload, CommandOutcome, ConfigReadPayload, ConfigReadResponse,
};
use crate::command::warm;
use anyhow::Result;

#[derive(Default)]
pub struct ConfigService;

impl ConfigService {
    pub async fn read(
        &self,
        payload: serde_json::Value,
        ctx: &CommandContext,
    ) -> Result<CommandOutcome> {
        let payload: ConfigReadPayload = parse_payload(payload)?;
        let project_ctx = ctx.resolve_project(payload.project).await?;
        let warm = warm::global_warmer().prewarm(&project_ctx.root).await;
        let mut outcome = CommandOutcome::from_value(ConfigReadResponse {
            config: project_ctx.config.clone(),
        })?;
        outcome.meta.config_path = project_ctx.config_path;
        outcome.meta.index_updated = Some(false);
        outcome.meta.warm = Some(warm.warmed);
        outcome.meta.warm_cost_ms = Some(warm.warm_cost_ms);
        outcome.meta.warm_graph_cache_hit = Some(warm.graph_cache_hit);
        outcome.hints.extend(project_ctx.hints);
        Ok(outcome)
    }
}
