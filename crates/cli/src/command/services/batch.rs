use super::Services;
use crate::command::context::CommandContext;
use crate::command::domain::{
    classify_error, parse_payload, BatchBudget, BatchItemResult, BatchOutput, BatchPayload,
    CommandAction, CommandOutcome, CommandStatus, Hint, HintKind, ResponseMeta, BATCH_VERSION,
};
use crate::command::freshness;
use anyhow::Result;
use context_batch_ref::resolve_batch_refs;
use context_protocol::{enforce_max_chars, finalize_used_chars, BudgetTruncation, ErrorEnvelope};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
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
            truncation: None,
        },
        next_actions: Vec::new(),
    };
    let min_chars = {
        let mut min_output = BatchOutput {
            version: BATCH_VERSION,
            items: Vec::new(),
            budget: BatchBudget {
                max_chars,
                used_chars: 0,
                truncated: true,
                truncation: Some(BudgetTruncation::MaxChars),
            },
            next_actions: Vec::new(),
        };
        finalize_batch_budget(&mut min_output)?
    };
    if min_chars > max_chars {
        return Err(anyhow::anyhow!(
            "max_chars too small for batch envelope (min_chars={min_chars})"
        ));
    }

    let mut inferred_project: Option<PathBuf> = payload.project;
    let mut gate: Option<freshness::FreshnessGate> = None;
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut ref_context = json!({
        "project": inferred_project.as_ref().map(|p| p.display().to_string()),
        "path": inferred_project.as_ref().map(|p| p.display().to_string()),
        "items": serde_json::Value::Object(serde_json::Map::new()),
    });

    for item in payload.items {
        let id = item.id.trim().to_string();
        if id.is_empty() {
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

        if !seen_ids.insert(id.clone()) {
            let rejected = error_item(
                id.clone(),
                format!("Duplicate batch item id is not supported: '{id}'"),
                Vec::new(),
                ResponseMeta::default(),
            );
            if !push_item_or_truncate(&mut output, rejected.clone())? {
                break;
            }

            ref_context["items"][id.clone()] = json!({
                "status": "error",
                "message": rejected.message,
                "data": rejected.data,
            });

            if payload.stop_on_error {
                break;
            }
            continue;
        }

        if matches!(item.action, CommandAction::Batch) {
            let rejected = error_item(
                id.clone(),
                "Nested batch actions are not supported".to_string(),
                Vec::new(),
                ResponseMeta::default(),
            );
            if !push_item_or_truncate(&mut output, rejected.clone())? {
                break;
            }

            ref_context["items"][id.clone()] = json!({
                "status": "error",
                "message": rejected.message,
                "data": rejected.data,
            });

            if payload.stop_on_error {
                break;
            }
            continue;
        }

        ref_context["project"] = inferred_project
            .as_ref()
            .map(|p| Value::String(p.display().to_string()))
            .unwrap_or(Value::Null);
        ref_context["path"] = ref_context["project"].clone();

        let resolved_payload = match resolve_batch_refs(item.payload, &ref_context) {
            Ok(value) => value,
            Err(err) => {
                let rejected = error_item(
                    id.clone(),
                    format!("Ref resolution error: {err}"),
                    Vec::new(),
                    ResponseMeta::default(),
                );
                if !push_item_or_truncate(&mut output, rejected.clone())? {
                    break;
                }

                ref_context["items"][id.clone()] = json!({
                    "status": "error",
                    "message": rejected.message,
                    "data": rejected.data,
                });

                if payload.stop_on_error {
                    break;
                }
                continue;
            }
        };

        let item_project = freshness::extract_project_path(&resolved_payload);
        if inferred_project.is_none() {
            inferred_project = item_project.clone();
        } else if let (Some(batch_project), Some(item_project)) = (&inferred_project, item_project)
        {
            if batch_project != &item_project {
                let rejected = error_item(
                    id.clone(),
                    format!(
                        "Batch project mismatch: batch uses '{}', item uses '{}'",
                        batch_project.display(),
                        item_project.display()
                    ),
                    Vec::new(),
                    ResponseMeta::default(),
                );
                if !push_item_or_truncate(&mut output, rejected.clone())? {
                    break;
                }

                ref_context["items"][id.clone()] = json!({
                    "status": "error",
                    "message": rejected.message,
                    "data": rejected.data,
                });

                if payload.stop_on_error {
                    break;
                }
                continue;
            }
        }

        ref_context["project"] = inferred_project
            .as_ref()
            .map(|p| Value::String(p.display().to_string()))
            .unwrap_or(Value::Null);
        ref_context["path"] = ref_context["project"].clone();

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
                    let rejected = error_item_with_context(
                        id.clone(),
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
                        Some(item.action),
                        Some(&resolved_payload),
                    );
                    if !push_item_or_truncate(&mut output, rejected.clone())? {
                        break;
                    }

                    ref_context["items"][id.clone()] = json!({
                        "status": "error",
                        "message": rejected.message,
                        "data": rejected.data,
                    });

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
            resolved_payload,
            inferred_project.as_ref(),
            &item.action,
            remaining_chars,
        );

        let item_payload_for_meta = item_payload.clone();
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
                    id: id.clone(),
                    status: CommandStatus::Ok,
                    message: None,
                    error: None,
                    hints: outcome.hints,
                    data: outcome.data,
                    meta: outcome.meta,
                }
            }
            Err(err) => {
                let message = format!("{err:#}");
                let classification =
                    classify_error(&message, Some(item.action), Some(&item_payload_for_meta));
                let mut hints = classification.hints;
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
                let hint = classification
                    .hint
                    .or_else(|| hints.first().map(|h| h.text.clone()));
                let error = ErrorEnvelope {
                    code: classification.code,
                    message: message.clone(),
                    details: None,
                    hint,
                    next_actions: classification.next_actions,
                };

                BatchItemResult {
                    id: id.clone(),
                    status: CommandStatus::Error,
                    message: Some(message),
                    error: Some(error),
                    hints,
                    data: Value::Null,
                    meta,
                }
            }
        };

        if !push_item_or_truncate(&mut output, item_outcome.clone())? {
            break;
        }

        let status = match item_outcome.status {
            CommandStatus::Ok => "ok",
            CommandStatus::Error => "error",
        };
        ref_context["items"][id.clone()] = json!({
            "status": status,
            "message": item_outcome.message,
            "data": item_outcome.data,
        });

        if payload.stop_on_error
            && output
                .items
                .last()
                .is_some_and(|v| v.status == CommandStatus::Error)
        {
            break;
        }
    }

    trim_batch_output(&mut output)?;

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
    let used = finalize_batch_budget(output)?;

    if used > output.budget.max_chars {
        let rejected = output.items.pop().expect("just pushed");
        output.budget.truncated = true;
        output.budget.truncation = Some(BudgetTruncation::MaxChars);

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
        } else {
            output.items.shrink_to_fit();
        }
        trim_batch_output(output)?;
        return Ok(false);
    }

    output.budget.used_chars = used;
    Ok(true)
}

fn finalize_batch_budget(output: &mut BatchOutput) -> Result<usize> {
    finalize_used_chars(output, |inner, used| inner.budget.used_chars = used)
}

fn error_item(
    id: String,
    message: String,
    hints: Vec<Hint>,
    meta: ResponseMeta,
) -> BatchItemResult {
    error_item_with_context(id, message, hints, meta, None, None)
}

fn error_item_with_context(
    id: String,
    message: String,
    hints: Vec<Hint>,
    meta: ResponseMeta,
    action: Option<CommandAction>,
    payload: Option<&Value>,
) -> BatchItemResult {
    let classification = classify_error(&message, action, payload);
    let mut out_hints = classification.hints;
    out_hints.extend(hints);
    let hint = classification
        .hint
        .or_else(|| out_hints.first().map(|h| h.text.clone()));
    let error = ErrorEnvelope {
        code: classification.code,
        message: message.clone(),
        details: None,
        hint,
        next_actions: classification.next_actions,
    };
    BatchItemResult {
        id,
        status: CommandStatus::Error,
        message: Some(message),
        error: Some(error),
        hints: out_hints,
        data: Value::Null,
        meta,
    }
}

fn trim_batch_output(output: &mut BatchOutput) -> Result<()> {
    let max_chars = output.budget.max_chars;
    let used = enforce_max_chars(
        output,
        max_chars,
        |inner, used| inner.budget.used_chars = used,
        |inner| {
            inner.budget.truncated = true;
            inner.budget.truncation = Some(BudgetTruncation::MaxChars);
        },
        |inner| {
            if !inner.items.is_empty() {
                inner.items.pop();
                return true;
            }
            false
        },
    )?;
    output.budget.used_chars = used;
    Ok(())
}
