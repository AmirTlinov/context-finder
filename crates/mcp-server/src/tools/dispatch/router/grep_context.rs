use super::super::{
    compute_grep_context_result, decode_cursor, finalize_grep_context_budget, CallToolResult,
    Content, ContextFinderService, GrepContextComputeOptions, GrepContextCursorV1,
    GrepContextRequest, McpError, CURSOR_VERSION,
};
use crate::tools::schemas::ToolNextAction;
use regex::RegexBuilder;
use serde_json::json;

use super::error::{
    internal_error_with_meta, invalid_cursor_with_meta, invalid_request_with_meta, meta_for_request,
};

fn build_regex(pattern: &str, case_sensitive: bool) -> Result<regex::Regex, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|err| format!("Invalid regex: {err}"))
}

struct CursorValidation<'a> {
    root_display: &'a str,
    pattern: &'a str,
    case_sensitive: bool,
    before: usize,
    after: usize,
    normalized_file: Option<&'a str>,
    normalized_file_pattern: Option<&'a str>,
}

fn decode_resume_cursor(
    cursor: Option<&str>,
    validation: &CursorValidation<'_>,
) -> Result<(Option<String>, usize), String> {
    let Some(cursor) = cursor else {
        return Ok((None, 1));
    };
    let cursor = cursor.trim();
    if cursor.is_empty() {
        return Ok((None, 1));
    }

    let decoded: GrepContextCursorV1 =
        decode_cursor(cursor).map_err(|err| format!("Invalid cursor: {err}"))?;
    if decoded.v != CURSOR_VERSION || decoded.tool != "grep_context" {
        return Err("Invalid cursor: wrong tool".to_string());
    }
    if decoded.root != validation.root_display {
        return Err("Invalid cursor: different root".to_string());
    }
    if decoded.pattern != validation.pattern {
        return Err("Invalid cursor: different pattern".to_string());
    }
    if decoded.file.as_deref() != validation.normalized_file {
        return Err("Invalid cursor: different file".to_string());
    }
    if decoded.file_pattern.as_deref() != validation.normalized_file_pattern {
        return Err("Invalid cursor: different file_pattern".to_string());
    }
    if decoded.case_sensitive != validation.case_sensitive
        || decoded.before != validation.before
        || decoded.after != validation.after
    {
        return Err("Invalid cursor: different search options".to_string());
    }
    Ok((Some(decoded.resume_file), decoded.resume_line.max(1)))
}

/// Regex search with merged context hunks (grep-like).
pub(in crate::tools::dispatch) async fn grep_context(
    service: &ContextFinderService,
    mut request: GrepContextRequest,
) -> Result<CallToolResult, McpError> {
    const DEFAULT_MAX_CHARS: usize = 20_000;
    const MAX_MAX_CHARS: usize = 500_000;
    const DEFAULT_MAX_MATCHES: usize = 2_000;
    const MAX_MAX_MATCHES: usize = 50_000;
    const DEFAULT_MAX_HUNKS: usize = 200;
    const MAX_MAX_HUNKS: usize = 50_000;
    const DEFAULT_CONTEXT: usize = 20;
    const MAX_CONTEXT: usize = 5_000;

    let (root, root_display) = match service.resolve_root(request.path.as_deref()).await {
        Ok(value) => value,
        Err(message) => {
            let meta = meta_for_request(service, request.path.as_deref()).await;
            return Ok(invalid_request_with_meta(message, meta, None, Vec::new()));
        }
    };
    let meta = service.tool_meta(&root).await;

    request.pattern = request.pattern.trim().to_string();
    if request.pattern.is_empty() {
        return Ok(invalid_request_with_meta(
            "Pattern must not be empty",
            meta.clone(),
            None,
            Vec::new(),
        ));
    }

    let case_sensitive = request.case_sensitive.unwrap_or(true);
    let regex = match build_regex(&request.pattern, case_sensitive) {
        Ok(re) => re,
        Err(msg) => {
            return Ok(invalid_request_with_meta(
                msg,
                meta.clone(),
                None,
                Vec::new(),
            ))
        }
    };

    let before = request
        .before
        .or(request.context)
        .unwrap_or(DEFAULT_CONTEXT)
        .clamp(0, MAX_CONTEXT);
    let after = request
        .after
        .or(request.context)
        .unwrap_or(DEFAULT_CONTEXT)
        .clamp(0, MAX_CONTEXT);

    let max_matches = request
        .max_matches
        .unwrap_or(DEFAULT_MAX_MATCHES)
        .clamp(1, MAX_MAX_MATCHES);
    let max_hunks = request
        .max_hunks
        .unwrap_or(DEFAULT_MAX_HUNKS)
        .clamp(1, MAX_MAX_HUNKS);
    let max_chars = request
        .max_chars
        .unwrap_or(DEFAULT_MAX_CHARS)
        .clamp(1, MAX_MAX_CHARS);

    let normalized_file = request
        .file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let normalized_file_pattern = request
        .file_pattern
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let (resume_file, resume_line) = match decode_resume_cursor(
        request.cursor.as_deref(),
        &CursorValidation {
            root_display: &root_display,
            pattern: &request.pattern,
            case_sensitive,
            before,
            after,
            normalized_file: normalized_file.as_deref(),
            normalized_file_pattern: normalized_file_pattern.as_deref(),
        },
    ) {
        Ok(v) => v,
        Err(msg) => return Ok(invalid_cursor_with_meta(msg, meta.clone())),
    };

    let mut result = match compute_grep_context_result(
        &root,
        &root_display,
        &request,
        &regex,
        GrepContextComputeOptions {
            case_sensitive,
            before,
            after,
            max_matches,
            max_hunks,
            max_chars,
            resume_file: resume_file.as_deref(),
            resume_line,
        },
    )
    .await
    {
        Ok(result) => result,
        Err(err) => {
            return Ok(internal_error_with_meta(
                format!("Error: {err:#}"),
                meta.clone(),
            ))
        }
    };
    result.meta = meta.clone();
    if let Some(cursor) = result.next_cursor.clone() {
        result.next_actions = Some(vec![ToolNextAction {
            tool: "grep_context".to_string(),
            args: json!({
                "path": root_display,
                "pattern": request.pattern,
                "file": normalized_file,
                "file_pattern": normalized_file_pattern,
                "before": before,
                "after": after,
                "case_sensitive": case_sensitive,
                "max_matches": max_matches,
                "max_hunks": max_hunks,
                "max_chars": max_chars,
                "cursor": cursor,
            }),
            reason: "Continue grep_context pagination with the next cursor.".to_string(),
        }]);
    }
    if let Err(err) = finalize_grep_context_budget(&mut result) {
        return Ok(invalid_request_with_meta(
            format!("max_chars too small for response envelope ({err:#})"),
            meta,
            None,
            Vec::new(),
        ));
    }

    Ok(CallToolResult::success(vec![Content::text(
        context_protocol::serialize_json(&result).unwrap_or_default(),
    )]))
}
