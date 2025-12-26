use super::super::{
    decode_cursor, encode_cursor, normalize_relative_path, CallToolResult, Content,
    ContextFinderService, FileScanner, McpError, TextSearchCursorModeV1, TextSearchCursorV1,
    TextSearchMatch, TextSearchRequest, TextSearchResult, CURSOR_VERSION,
};
use crate::tools::schemas::ToolNextAction;
use context_vector_store::ChunkCorpus;
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 2_000_000;

use super::error::{internal_error, invalid_cursor, invalid_request};

fn trimmed_non_empty_str(input: Option<&str>) -> Option<&str> {
    input.map(str::trim).filter(|value| !value.is_empty())
}

struct TextSearchSettings<'a> {
    pattern: &'a str,
    file_pattern: Option<&'a str>,
    max_results: usize,
    case_sensitive: bool,
    whole_word: bool,
}

struct TextSearchOutcome {
    matches: Vec<TextSearchMatch>,
    matched_files: HashSet<String>,
    seen: HashSet<TextSearchKey>,
    scanned_files: usize,
    skipped_large_files: usize,
    truncated: bool,
    next_state: Option<TextSearchCursorModeV1>,
}

#[derive(Hash, PartialEq, Eq)]
struct TextSearchKey {
    file: String,
    line: usize,
    column: usize,
    text: String,
}

impl TextSearchOutcome {
    fn new() -> Self {
        Self {
            matches: Vec::new(),
            matched_files: HashSet::new(),
            seen: HashSet::new(),
            scanned_files: 0,
            skipped_large_files: 0,
            truncated: false,
            next_state: None,
        }
    }

    fn push_match(&mut self, item: TextSearchMatch) -> bool {
        let key = TextSearchKey {
            file: item.file.clone(),
            line: item.line,
            column: item.column,
            text: item.text.clone(),
        };
        if !self.seen.insert(key) {
            return false;
        }
        self.matched_files.insert(item.file.clone());
        self.matches.push(item);
        true
    }
}

fn decode_cursor_mode(
    request: &TextSearchRequest,
    root_display: &str,
    settings: &TextSearchSettings<'_>,
    normalized_file_pattern: Option<&String>,
) -> std::result::Result<Option<TextSearchCursorModeV1>, CallToolResult> {
    let Some(cursor) = request
        .cursor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };

    let decoded: TextSearchCursorV1 = match decode_cursor(cursor) {
        Ok(v) => v,
        Err(err) => return Err(invalid_cursor(format!("Invalid cursor: {err}"))),
    };

    if decoded.v != CURSOR_VERSION || decoded.tool != "text_search" {
        return Err(invalid_cursor("Invalid cursor: wrong tool"));
    }
    if decoded.root != root_display {
        return Err(invalid_cursor("Invalid cursor: different root"));
    }
    if decoded.pattern != settings.pattern {
        return Err(invalid_cursor("Invalid cursor: different pattern"));
    }
    if decoded.file_pattern.as_ref() != normalized_file_pattern {
        return Err(invalid_cursor("Invalid cursor: different file_pattern"));
    }
    if decoded.case_sensitive != settings.case_sensitive
        || decoded.whole_word != settings.whole_word
    {
        return Err(invalid_cursor("Invalid cursor: different search options"));
    }

    Ok(Some(decoded.mode))
}

fn start_indices_for_corpus(
    cursor_mode: Option<&TextSearchCursorModeV1>,
) -> std::result::Result<(usize, usize, usize), CallToolResult> {
    match cursor_mode {
        None => Ok((0, 0, 0)),
        Some(TextSearchCursorModeV1::Corpus {
            file_index,
            chunk_index,
            line_offset,
        }) => Ok((*file_index, *chunk_index, *line_offset)),
        Some(TextSearchCursorModeV1::Filesystem { .. }) => {
            Err(invalid_cursor("Invalid cursor: wrong mode"))
        }
    }
}

fn start_indices_for_filesystem(
    cursor_mode: Option<&TextSearchCursorModeV1>,
) -> std::result::Result<(usize, usize), CallToolResult> {
    match cursor_mode {
        None => Ok((0, 0)),
        Some(TextSearchCursorModeV1::Filesystem {
            file_index,
            line_offset,
        }) => Ok((*file_index, *line_offset)),
        Some(TextSearchCursorModeV1::Corpus { .. }) => {
            Err(invalid_cursor("Invalid cursor: wrong mode"))
        }
    }
}

fn encode_next_cursor(
    root_display: &str,
    settings: &TextSearchSettings<'_>,
    normalized_file_pattern: Option<&String>,
    mode: TextSearchCursorModeV1,
) -> std::result::Result<String, CallToolResult> {
    let token = TextSearchCursorV1 {
        v: CURSOR_VERSION,
        tool: "text_search".to_string(),
        root: root_display.to_string(),
        pattern: settings.pattern.to_string(),
        file_pattern: normalized_file_pattern.cloned(),
        case_sensitive: settings.case_sensitive,
        whole_word: settings.whole_word,
        mode,
    };

    encode_cursor(&token).map_err(|err| internal_error(format!("Error: {err:#}")))
}

fn search_in_corpus(
    corpus: &ChunkCorpus,
    settings: &TextSearchSettings<'_>,
    start_file_index: usize,
    start_chunk_index: usize,
    start_line_offset: usize,
) -> std::result::Result<TextSearchOutcome, CallToolResult> {
    let mut outcome = TextSearchOutcome::new();

    let mut files: Vec<(&String, &Vec<context_code_chunker::CodeChunk>)> =
        corpus.files().iter().collect();
    files.sort_by(|a, b| a.0.cmp(b.0));
    files.retain(|(file, _)| {
        ContextFinderService::matches_file_pattern(file, settings.file_pattern)
    });

    if start_file_index > files.len() {
        return Err(invalid_cursor("Invalid cursor: out of range"));
    }

    'outer_corpus: for (file_index, (_file, chunks)) in
        files.iter().enumerate().skip(start_file_index)
    {
        if outcome.matches.len() >= settings.max_results {
            outcome.truncated = true;
            outcome.next_state = Some(TextSearchCursorModeV1::Corpus {
                file_index,
                chunk_index: 0,
                line_offset: 0,
            });
            break 'outer_corpus;
        }

        outcome.scanned_files += 1;

        let mut chunk_refs: Vec<&context_code_chunker::CodeChunk> = chunks.iter().collect();
        chunk_refs.sort_by(|a, b| {
            a.start_line
                .cmp(&b.start_line)
                .then_with(|| a.end_line.cmp(&b.end_line))
        });

        let first_file = file_index == start_file_index;
        let start_chunk = if first_file { start_chunk_index } else { 0 };
        if start_chunk > chunk_refs.len() {
            return Err(invalid_cursor("Invalid cursor: out of range"));
        }

        for (chunk_index, chunk) in chunk_refs.iter().enumerate().skip(start_chunk) {
            if outcome.matches.len() >= settings.max_results {
                outcome.truncated = true;
                outcome.next_state = Some(TextSearchCursorModeV1::Corpus {
                    file_index,
                    chunk_index,
                    line_offset: 0,
                });
                break 'outer_corpus;
            }

            let line_start = if first_file && chunk_index == start_chunk {
                start_line_offset
            } else {
                0
            };

            for (offset, line_text) in chunk.content.lines().enumerate().skip(line_start) {
                if outcome.matches.len() >= settings.max_results {
                    outcome.truncated = true;
                    outcome.next_state = Some(TextSearchCursorModeV1::Corpus {
                        file_index,
                        chunk_index,
                        line_offset: offset,
                    });
                    break 'outer_corpus;
                }

                let Some(col_byte) = ContextFinderService::match_in_line(
                    line_text,
                    settings.pattern,
                    settings.case_sensitive,
                    settings.whole_word,
                ) else {
                    continue;
                };

                let line = chunk.start_line + offset;
                let column = line_text[..col_byte].chars().count() + 1;
                let _ = outcome.push_match(TextSearchMatch {
                    file: chunk.file_path.clone(),
                    line,
                    column,
                    text: line_text.to_string(),
                });
            }
        }
    }

    Ok(outcome)
}

fn search_in_filesystem(
    root: &Path,
    settings: &TextSearchSettings<'_>,
    start_file_index: usize,
    start_line_offset: usize,
) -> std::result::Result<TextSearchOutcome, CallToolResult> {
    let mut outcome = TextSearchOutcome::new();

    let scanner = FileScanner::new(root);
    let mut candidates: Vec<(String, PathBuf)> = scanner
        .scan()
        .into_iter()
        .filter_map(|file| normalize_relative_path(root, &file).map(|rel| (rel, file)))
        .filter(|(rel, _)| ContextFinderService::matches_file_pattern(rel, settings.file_pattern))
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));

    if start_file_index > candidates.len() {
        return Err(invalid_cursor("Invalid cursor: out of range"));
    }

    'outer_fs: for (file_index, (rel_path, abs_path)) in
        candidates.iter().enumerate().skip(start_file_index)
    {
        if outcome.matches.len() >= settings.max_results {
            outcome.truncated = true;
            outcome.next_state = Some(TextSearchCursorModeV1::Filesystem {
                file_index,
                line_offset: 0,
            });
            break 'outer_fs;
        }

        outcome.scanned_files += 1;

        let Ok(meta) = std::fs::metadata(abs_path) else {
            continue;
        };
        if meta.len() > MAX_FILE_BYTES {
            outcome.skipped_large_files += 1;
            continue;
        }

        let Ok(content) = std::fs::read_to_string(abs_path) else {
            continue;
        };

        let first_file = file_index == start_file_index;
        let line_start = if first_file { start_line_offset } else { 0 };

        for (offset, line_text) in content.lines().enumerate().skip(line_start) {
            if outcome.matches.len() >= settings.max_results {
                outcome.truncated = true;
                outcome.next_state = Some(TextSearchCursorModeV1::Filesystem {
                    file_index,
                    line_offset: offset,
                });
                break 'outer_fs;
            }

            let Some(col_byte) = ContextFinderService::match_in_line(
                line_text,
                settings.pattern,
                settings.case_sensitive,
                settings.whole_word,
            ) else {
                continue;
            };
            let column = line_text[..col_byte].chars().count() + 1;
            let _ = outcome.push_match(TextSearchMatch {
                file: rel_path.clone(),
                line: offset + 1,
                column,
                text: line_text.to_string(),
            });
        }
    }

    Ok(outcome)
}

/// Bounded exact text search (literal substring), as a safe `rg` replacement.
pub(in crate::tools::dispatch) async fn text_search(
    service: &ContextFinderService,
    request: TextSearchRequest,
) -> Result<CallToolResult, McpError> {
    let (root, root_display) = match service.resolve_root(request.path.as_deref()).await {
        Ok(value) => value,
        Err(message) => return Ok(invalid_request(message)),
    };

    let pattern = request.pattern.trim();
    if pattern.is_empty() {
        return Ok(invalid_request("Pattern must not be empty"));
    }

    let file_pattern = trimmed_non_empty_str(request.file_pattern.as_deref());
    let max_results = request.max_results.unwrap_or(50).clamp(1, 1000);
    let case_sensitive = request.case_sensitive.unwrap_or(true);
    let whole_word = request.whole_word.unwrap_or(false);
    let normalized_file_pattern = file_pattern.map(str::to_string);
    let settings = TextSearchSettings {
        pattern,
        file_pattern,
        max_results,
        case_sensitive,
        whole_word,
    };

    let cursor_mode = match decode_cursor_mode(
        &request,
        &root_display,
        &settings,
        normalized_file_pattern.as_ref(),
    ) {
        Ok(value) => value,
        Err(result) => return Ok(result),
    };

    let corpus = match ContextFinderService::load_chunk_corpus(&root).await {
        Ok(corpus) => corpus,
        Err(err) => return Ok(internal_error(format!("Error: {err:#}"))),
    };

    let (source, mut outcome) = if let Some(corpus) = corpus {
        let (start_file_index, start_chunk_index, start_line_offset) =
            match start_indices_for_corpus(cursor_mode.as_ref()) {
                Ok(value) => value,
                Err(result) => return Ok(result),
            };
        let outcome = match search_in_corpus(
            &corpus,
            &settings,
            start_file_index,
            start_chunk_index,
            start_line_offset,
        ) {
            Ok(value) => value,
            Err(result) => return Ok(result),
        };
        ("corpus".to_string(), outcome)
    } else {
        let (start_file_index, start_line_offset) =
            match start_indices_for_filesystem(cursor_mode.as_ref()) {
                Ok(value) => value,
                Err(result) => return Ok(result),
            };
        let outcome =
            match search_in_filesystem(&root, &settings, start_file_index, start_line_offset) {
                Ok(value) => value,
                Err(result) => return Ok(result),
            };
        ("filesystem".to_string(), outcome)
    };

    let next_cursor = if outcome.truncated {
        let Some(mode) = outcome.next_state.take() else {
            return Ok(internal_error("Internal error: missing cursor state"));
        };
        match encode_next_cursor(
            &root_display,
            &settings,
            normalized_file_pattern.as_ref(),
            mode,
        ) {
            Ok(value) => Some(value),
            Err(result) => return Ok(result),
        }
    } else {
        None
    };

    let mut result = TextSearchResult {
        pattern: settings.pattern.to_string(),
        source,
        scanned_files: outcome.scanned_files,
        matched_files: outcome.matched_files.len(),
        skipped_large_files: outcome.skipped_large_files,
        returned: outcome.matches.len(),
        truncated: outcome.truncated,
        next_cursor,
        next_actions: None,
        meta: None,
        matches: outcome.matches,
    };
    result.meta = Some(service.tool_meta(&root).await);
    if let Some(cursor) = result.next_cursor.clone() {
        result.next_actions = Some(vec![ToolNextAction {
            tool: "text_search".to_string(),
            args: json!({
                "path": root_display,
                "pattern": settings.pattern,
                "file_pattern": normalized_file_pattern,
                "max_results": max_results,
                "case_sensitive": settings.case_sensitive,
                "whole_word": settings.whole_word,
                "cursor": cursor,
            }),
            reason: "Continue text_search pagination with the next cursor.".to_string(),
        }]);
    }

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&result).unwrap_or_default(),
    )]))
}

#[cfg(test)]
mod tests {
    use super::{TextSearchMatch, TextSearchOutcome};

    #[test]
    fn text_search_dedupes_matches() {
        let mut outcome = TextSearchOutcome::new();
        let first = TextSearchMatch {
            file: "src/main.rs".to_string(),
            line: 1,
            column: 1,
            text: "fn main() {}".to_string(),
        };
        assert!(outcome.push_match(first));

        let dup = TextSearchMatch {
            file: "src/main.rs".to_string(),
            line: 1,
            column: 1,
            text: "fn main() {}".to_string(),
        };
        assert!(!outcome.push_match(dup));
        assert_eq!(outcome.matches.len(), 1);
        assert_eq!(outcome.matched_files.len(), 1);
    }
}
