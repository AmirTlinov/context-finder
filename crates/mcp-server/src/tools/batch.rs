use anyhow::Result;
use rmcp::model::CallToolResult;

use super::schemas::batch::{
    BatchBudget, BatchItemResult, BatchItemStatus, BatchResult, BatchToolName,
};

pub(super) fn resolve_batch_refs(
    input: serde_json::Value,
    ctx: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    context_batch_ref::resolve_batch_refs(input, ctx)
}

pub(super) fn extract_path_from_input(input: &serde_json::Value) -> Option<String> {
    let serde_json::Value::Object(map) = input else {
        return None;
    };
    map.get("path")
        .or_else(|| map.get("project"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

pub(super) fn prepare_item_input(
    input: serde_json::Value,
    path: Option<&str>,
    tool: BatchToolName,
    remaining_chars: usize,
) -> serde_json::Value {
    let mut input = match input {
        serde_json::Value::Object(map) => serde_json::Value::Object(map),
        _ => serde_json::Value::Object(serde_json::Map::new()),
    };

    if let Some(path) = path {
        if let serde_json::Value::Object(ref mut map) = input {
            map.entry("path".to_string())
                .or_insert_with(|| serde_json::Value::String(path.to_string()));
        }
    }

    if matches!(
        tool,
        BatchToolName::ContextPack
            | BatchToolName::FileSlice
            | BatchToolName::ListFiles
            | BatchToolName::GrepContext
    ) {
        if let serde_json::Value::Object(ref mut map) = input {
            if !map.contains_key("max_chars") {
                let cap = remaining_chars.saturating_sub(300).clamp(1, 20_000);
                map.insert(
                    "max_chars".to_string(),
                    serde_json::Value::Number(cap.into()),
                );
            }
        }
    }

    input
}

pub(super) fn parse_tool_result_as_json(
    result: &CallToolResult,
    tool: BatchToolName,
) -> Result<serde_json::Value, String> {
    if result.is_error.unwrap_or(false) {
        return Err(extract_tool_text(result).unwrap_or_else(|| "Tool returned error".to_string()));
    }

    if let Some(value) = result.structured_content.clone() {
        return Ok(value);
    }

    let blocks = extract_tool_text_blocks(result);
    if blocks.is_empty() {
        return Err("Tool returned no text content".to_string());
    }

    let mut parsed = Vec::new();
    for block in blocks {
        match serde_json::from_str::<serde_json::Value>(&block) {
            Ok(v) => parsed.push(v),
            Err(err) => {
                return Err(format!("Tool returned non-JSON text content: {err}"));
            }
        }
    }

    match parsed.len() {
        1 => Ok(parsed.into_iter().next().expect("len=1")),
        2 if matches!(tool, BatchToolName::ContextPack) => Ok(serde_json::json!({
            "result": parsed[0],
            "trace": parsed[1],
        })),
        _ => Ok(serde_json::Value::Array(parsed)),
    }
}

fn extract_tool_text(result: &CallToolResult) -> Option<String> {
    let blocks = extract_tool_text_blocks(result);
    if blocks.is_empty() {
        return None;
    }
    Some(blocks.join("\n"))
}

fn extract_tool_text_blocks(result: &CallToolResult) -> Vec<String> {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect()
}

pub(super) fn push_item_or_truncate(output: &mut BatchResult, item: BatchItemResult) -> bool {
    output.items.push(item);
    let used = match compute_used_chars(output) {
        Ok(used) => used,
        Err(err) => {
            let rejected = output.items.pop().expect("just pushed");
            output.budget.truncated = true;
            output.items.push(BatchItemResult {
                id: rejected.id,
                tool: rejected.tool,
                status: BatchItemStatus::Error,
                message: Some(format!("Failed to compute batch budget: {err:#}")),
                data: serde_json::Value::Null,
            });
            return false;
        }
    };

    if used > output.budget.max_chars {
        let rejected = output.items.pop().expect("just pushed");
        output.budget.truncated = true;

        if output.items.is_empty() {
            output.items.push(BatchItemResult {
                id: rejected.id,
                tool: rejected.tool,
                status: BatchItemStatus::Error,
                message: Some(format!(
                    "Batch budget exceeded (max_chars={}). Reduce payload sizes or raise max_chars.",
                    output.budget.max_chars
                )),
                data: serde_json::Value::Null,
            });
        }

        output.budget.used_chars = compute_used_chars(output).unwrap_or(output.budget.max_chars);
        return false;
    }

    output.budget.used_chars = used;
    true
}

pub(super) fn compute_used_chars(output: &BatchResult) -> anyhow::Result<usize> {
    let mut tmp = BatchResult {
        version: output.version,
        items: output.items.clone(),
        budget: BatchBudget {
            max_chars: output.budget.max_chars,
            used_chars: 0,
            truncated: output.budget.truncated,
        },
        meta: output.meta.clone(),
    };
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
