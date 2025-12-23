use super::Services;
use crate::command::context::CommandContext;
use crate::command::domain::{
    classify_error, parse_payload, BatchBudget, BatchItemResult, BatchOutput, BatchPayload,
    CommandAction, CommandOutcome, CommandStatus, Hint, HintKind, ResponseMeta, BATCH_VERSION,
};
use crate::command::freshness;
use anyhow::Result;
use serde_json::{Map, Value};
use std::path::PathBuf;

const DEFAULT_BATCH_MAX_CHARS: usize = 20_000;
const MAX_BATCH_MAX_CHARS: usize = 500_000;

pub async fn run(
    services: &Services,
    payload: Value,
    ctx: &CommandContext,
) -> Result<CommandOutcome> {
    let payload: BatchPayload = parse_payload(payload)?;

    let max_chars = payload
        .max_chars
        .unwrap_or(DEFAULT_BATCH_MAX_CHARS)
        .clamp(1, MAX_BATCH_MAX_CHARS);

    let mut output = BatchOutput {
        version: BATCH_VERSION,
        items: Vec::new(),
        budget: BatchBudget {
            max_chars,
            used_chars: 0,
            truncated: false,
        },
    };

    let mut inferred_project: Option<PathBuf> = payload.project;
    let mut gate: Option<freshness::FreshnessGate> = None;

    for item in payload.items {
        if item.id.trim().is_empty() {
            let rejected = error_item(
                item.id,
                "Batch item id must not be empty".to_string(),
                Vec::new(),
                ResponseMeta::default(),
            );
            if !push_item_or_truncate(&mut output, rejected)? {
                break;
            }
            if payload.stop_on_error {
                break;
            }
            continue;
        }

        if matches!(item.action, CommandAction::Batch) {
            let rejected = error_item(
                item.id,
                "Nested batch actions are not supported".to_string(),
                Vec::new(),
                ResponseMeta::default(),
            );
            if !push_item_or_truncate(&mut output, rejected)? {
                break;
            }
            if payload.stop_on_error {
                break;
            }
            continue;
        }

        let item_project = freshness::extract_project_path(&item.payload);
        if inferred_project.is_none() {
            inferred_project = item_project.clone();
        } else if let (Some(batch_project), Some(item_project)) = (&inferred_project, item_project)
        {
            if batch_project != &item_project {
                let rejected = error_item(
                    item.id,
                    format!(
                        "Batch project mismatch: batch uses '{}', item uses '{}'",
                        batch_project.display(),
                        item_project.display()
                    ),
                    Vec::new(),
                    ResponseMeta::default(),
                );
                if !push_item_or_truncate(&mut output, rejected)? {
                    break;
                }
                if payload.stop_on_error {
                    break;
                }
                continue;
            }
        }

        let requires_index = freshness::action_requires_index(&item.action);
        if requires_index && gate.is_none() {
            let project_ctx = ctx.resolve_project(inferred_project.clone()).await?;
            match freshness::enforce_stale_policy(
                &project_ctx.root,
                &project_ctx.profile_name,
                &project_ctx.profile,
                &ctx.request_options(),
            )
            .await?
            {
                Ok(new_gate) => gate = Some(new_gate),
                Err(block) => {
                    let mut hints = block.hints;
                    hints.extend(project_ctx.hints);
                    hints.extend(classify_error(&block.message));
                    let rejected = error_item(
                        item.id,
                        block.message,
                        hints,
                        ResponseMeta {
                            config_path: project_ctx.config_path,
                            profile: Some(project_ctx.profile_name),
                            profile_path: project_ctx.profile_path,
                            index_state: Some(block.index_state),
                            index_updated: Some(false),
                            ..Default::default()
                        },
                    );
                    if !push_item_or_truncate(&mut output, rejected)? {
                        break;
                    }
                    if payload.stop_on_error {
                        break;
                    }
                    continue;
                }
            }
        }

        let remaining_chars = output
            .budget
            .max_chars
            .saturating_sub(output.budget.used_chars);
        let item_payload = prepare_item_payload(
            item.payload,
            inferred_project.as_ref(),
            &item.action,
            remaining_chars,
        );

        let item_outcome = match services.route_item(item.action, item_payload, ctx).await {
            Ok(mut outcome) => {
                if matches!(item.action, CommandAction::Index) {
                    let project_ctx = ctx.resolve_project(inferred_project.clone()).await?;
                    if let Ok(state) =
                        freshness::gather_index_state(&project_ctx.root, &project_ctx.profile_name)
                            .await
                    {
                        outcome.meta.index_state = Some(state);
                    }
                    gate = None;
                } else if requires_index {
                    if let Some(ref gate) = gate {
                        if outcome.meta.index_state.is_none() {
                            outcome.meta.index_state = Some(gate.index_state.clone());
                        }
                        if gate.index_updated {
                            outcome.meta.index_updated = Some(true);
                        }
                        outcome.hints.extend(gate.hints.clone());
                    }
                }

                BatchItemResult {
                    id: item.id,
                    status: CommandStatus::Ok,
                    message: None,
                    hints: outcome.hints,
                    data: outcome.data,
                    meta: outcome.meta,
                }
            }
            Err(err) => {
                let message = format!("{err:#}");
                let mut hints = classify_error(&message);
                let mut meta = ResponseMeta::default();
                if requires_index {
                    if let Some(ref gate) = gate {
                        hints.extend(gate.hints.clone());
                        meta.index_state = Some(gate.index_state.clone());
                        if gate.index_updated {
                            meta.index_updated = Some(true);
                        }
                    }
                }

                BatchItemResult {
                    id: item.id,
                    status: CommandStatus::Error,
                    message: Some(message),
                    hints,
                    data: Value::Null,
                    meta,
                }
            }
        };

        if !push_item_or_truncate(&mut output, item_outcome)? {
            break;
        }

        if payload.stop_on_error
            && output
                .items
                .last()
                .is_some_and(|v| v.status == CommandStatus::Error)
        {
            break;
        }
    }

    if output.budget.truncated {
        output.items.shrink_to_fit();
    }

    let mut outcome = CommandOutcome::from_value(output.clone())?;
    if output.budget.truncated {
        outcome.hints.push(Hint {
            kind: HintKind::Warn,
            text: format!(
                "Batch output truncated at ~{} chars (max_chars={})",
                output.budget.used_chars, output.budget.max_chars
            ),
        });
    }
    Ok(outcome)
}

fn prepare_item_payload(
    payload: Value,
    project: Option<&PathBuf>,
    action: &CommandAction,
    remaining_chars: usize,
) -> Value {
    let mut payload = match payload {
        Value::Object(map) => Value::Object(map),
        _ => Value::Object(Map::new()),
    };

    if let Some(project) = project {
        let project_str = project.display().to_string();
        if payload.get("project").is_none() {
            payload["project"] = Value::String(project_str.clone());
        }
        if payload.get("path").is_none() {
            payload["path"] = Value::String(project_str);
        }
    }

    if matches!(action, CommandAction::ContextPack | CommandAction::TaskPack)
        && payload.get("max_chars").is_none()
    {
        let cap = remaining_chars.saturating_sub(300).clamp(1, 20_000);
        payload["max_chars"] = Value::Number(cap.into());
    }

    payload
}

fn push_item_or_truncate(output: &mut BatchOutput, item: BatchItemResult) -> Result<bool> {
    output.items.push(item);
    let used = compute_used_chars(output)?;

    if used > output.budget.max_chars {
        let rejected = output.items.pop().expect("just pushed");
        output.budget.truncated = true;

        if output.items.is_empty() {
            output.items.push(error_item(
                rejected.id,
                format!(
                    "Batch budget exceeded (max_chars={}). Reduce payload sizes or raise max_chars.",
                    output.budget.max_chars
                ),
                Vec::new(),
                ResponseMeta::default(),
            ));
        }

        output.budget.used_chars = compute_used_chars(output)?;
        return Ok(false);
    }

    output.budget.used_chars = used;
    Ok(true)
}

fn compute_used_chars(output: &BatchOutput) -> Result<usize> {
    let mut tmp = output.clone();
    tmp.budget.used_chars = 0;
    let raw = serde_json::to_string(&tmp)?;
    let mut used = raw.chars().count();
    tmp.budget.used_chars = used;
    let raw = serde_json::to_string(&tmp)?;
    let next = raw.chars().count();
    if next == used {
        return Ok(used);
    }
    used = next;
    tmp.budget.used_chars = used;
    let raw = serde_json::to_string(&tmp)?;
    Ok(raw.chars().count())
}

fn error_item(
    id: String,
    message: String,
    hints: Vec<Hint>,
    meta: ResponseMeta,
) -> BatchItemResult {
    let mut out_hints = classify_error(&message);
    out_hints.extend(hints);
    BatchItemResult {
        id,
        status: CommandStatus::Error,
        message: Some(message),
        hints: out_hints,
        data: Value::Null,
        meta,
    }
}
