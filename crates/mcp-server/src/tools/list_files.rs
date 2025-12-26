use anyhow::{Context as AnyhowContext, Result};
use context_indexer::FileScanner;
use std::path::Path;

use super::cursor::{encode_cursor, CURSOR_VERSION};
use super::paths::normalize_relative_path;
use super::schemas::list_files::{ListFilesCursorV1, ListFilesResult, ListFilesTruncation};
use super::ContextFinderService;

pub(super) fn decode_list_files_cursor(cursor: &str) -> Result<ListFilesCursorV1> {
    super::cursor::decode_cursor(cursor).with_context(|| "decode list_files cursor")
}

pub(super) async fn compute_list_files_result(
    root: &Path,
    root_display: &str,
    file_pattern: Option<&str>,
    limit: usize,
    max_chars: usize,
    cursor_last_file: Option<&str>,
) -> Result<ListFilesResult> {
    let file_pattern = file_pattern.map(str::trim).filter(|s| !s.is_empty());
    let cursor_last_file = cursor_last_file.map(str::trim).filter(|s| !s.is_empty());

    let mut used_chars = 0usize;
    let mut truncated = false;
    let mut truncation: Option<ListFilesTruncation> = None;
    let mut files: Vec<String> = Vec::new();
    let mut next_cursor: Option<String> = None;
    let source: String;
    let scanned_files: usize;
    let mut matched: Vec<String> = Vec::new();

    if let Some(corpus) = ContextFinderService::load_chunk_corpus(root).await? {
        source = "corpus".to_string();

        let mut candidates: Vec<&String> = corpus.files().keys().collect();
        candidates.sort();
        scanned_files = candidates.len();

        for file in candidates {
            if !ContextFinderService::matches_file_pattern(file, file_pattern) {
                continue;
            }
            matched.push(file.clone());
        }
    } else {
        source = "filesystem".to_string();

        let scanner = FileScanner::new(root);
        let scanned_paths = scanner.scan();
        scanned_files = scanned_paths.len();

        let mut candidates: Vec<String> = scanned_paths
            .into_iter()
            .filter_map(|p| normalize_relative_path(root, &p))
            .collect();
        candidates.sort();

        for file in candidates {
            if !ContextFinderService::matches_file_pattern(&file, file_pattern) {
                continue;
            }
            matched.push(file);
        }
    }

    let start_index = cursor_last_file.map_or(0, |last| {
        match matched.binary_search_by(|candidate| candidate.as_str().cmp(last)) {
            Ok(idx) => idx + 1,
            Err(idx) => idx,
        }
    });

    if start_index > matched.len() {
        anyhow::bail!("Cursor is out of range for matched files");
    }

    for file in matched.iter().skip(start_index) {
        if files.len() >= limit {
            truncated = true;
            truncation = Some(ListFilesTruncation::Limit);
            break;
        }

        let file_chars = file.chars().count();
        let extra_chars = if files.is_empty() {
            file_chars
        } else {
            1 + file_chars
        };
        if used_chars.saturating_add(extra_chars) > max_chars {
            truncated = true;
            truncation = Some(ListFilesTruncation::MaxChars);
            break;
        }

        files.push(file.clone());
        used_chars += extra_chars;
    }

    if truncated && !files.is_empty() && start_index.saturating_add(files.len()) < matched.len() {
        if let Some(last_file) = files.last() {
            next_cursor = Some(encode_cursor(&ListFilesCursorV1 {
                v: CURSOR_VERSION,
                tool: "list_files".to_string(),
                root: root_display.to_string(),
                file_pattern: file_pattern.map(str::to_string),
                last_file: last_file.clone(),
            })?);
        }
    }

    Ok(ListFilesResult {
        source,
        file_pattern: file_pattern.map(str::to_string),
        scanned_files,
        returned: files.len(),
        used_chars,
        limit,
        max_chars,
        truncated,
        truncation,
        next_cursor,
        next_actions: None,
        meta: None,
        files,
    })
}
