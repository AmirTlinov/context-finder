use crate::command::context::CommandContext;
use crate::command::domain::{
    parse_payload, CommandOutcome, Hint, HintKind, TextSearchMatch, TextSearchOutput,
    TextSearchPayload,
};
use crate::command::warm;
use anyhow::{anyhow, Result};
use context_indexer::FileScanner;
use context_protocol::ToolNextAction;
use context_vector_store::{corpus_path_for_project_root, ChunkCorpus};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::Path;

#[derive(Default)]
pub struct TextSearchService;

impl TextSearchService {
    pub async fn run(
        &self,
        payload: serde_json::Value,
        ctx: &CommandContext,
    ) -> Result<CommandOutcome> {
        let payload: TextSearchPayload = parse_payload(payload)?;

        let pattern = payload.pattern.trim();
        if pattern.is_empty() {
            return Err(anyhow!("Pattern must not be empty"));
        }

        let max_results = payload.max_results.unwrap_or(50).clamp(1, 1000);
        let case_sensitive = payload.case_sensitive.unwrap_or(true);
        let whole_word = payload.whole_word.unwrap_or(false);

        let request_options = ctx.request_options();
        let file_pattern = request_options
            .file_pattern
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty());

        const MAX_FILE_BYTES: u64 = 2_000_000;

        let project_ctx = ctx.resolve_project(payload.project).await?;
        let _ = crate::heartbeat::ping(&project_ctx.root).await;
        let warm = warm::global_warmer().prewarm(&project_ctx.root).await;

        let mut matches: Vec<TextSearchMatch> = Vec::new();
        let mut matched_files: HashSet<String> = HashSet::new();
        let mut scanned_files = 0usize;
        let mut skipped_large_files = 0usize;
        let mut truncated = false;
        let source: String;

        let corpus = load_chunk_corpus(&project_ctx.root).await?;
        if let Some(corpus) = corpus {
            source = "corpus".to_string();

            let mut files: Vec<(&String, &Vec<context_code_chunker::CodeChunk>)> =
                corpus.files().iter().collect();
            files.sort_by(|a, b| a.0.cmp(b.0));

            'outer_corpus: for (file, chunks) in files {
                if matches.len() >= max_results {
                    truncated = true;
                    break 'outer_corpus;
                }
                if !crate::command::path_filters::path_allowed(file, &request_options) {
                    continue;
                }
                scanned_files += 1;

                for chunk in chunks {
                    for (offset, line_text) in chunk.content.lines().enumerate() {
                        if matches.len() >= max_results {
                            truncated = true;
                            break 'outer_corpus;
                        }
                        let Some(col_byte) =
                            match_in_line(line_text, pattern, case_sensitive, whole_word)
                        else {
                            continue;
                        };

                        let line = chunk.start_line + offset;
                        let column = line_text[..col_byte].chars().count() + 1;
                        matched_files.insert(chunk.file_path.clone());
                        matches.push(TextSearchMatch {
                            file: chunk.file_path.clone(),
                            line,
                            column,
                            text: line_text.to_string(),
                        });
                    }
                }
            }
        } else {
            source = "filesystem".to_string();
            if !request_options.allow_filesystem_fallback {
                return Err(anyhow!(
                    "Chunk corpus missing and filesystem fallback is disabled (options.allow_filesystem_fallback=false)"
                ));
            }

            let scanner = FileScanner::new(&project_ctx.root);
            let files = scanner.scan();

            'outer_fs: for file in files {
                if matches.len() >= max_results {
                    truncated = true;
                    break 'outer_fs;
                }
                let Some(rel_path) = normalize_relative_path(&project_ctx.root, &file) else {
                    continue;
                };
                if !crate::command::path_filters::path_allowed(&rel_path, &request_options) {
                    continue;
                }

                scanned_files += 1;
                let meta = match std::fs::metadata(&file) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if meta.len() > MAX_FILE_BYTES {
                    skipped_large_files += 1;
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&file) else {
                    continue;
                };

                for (offset, line_text) in content.lines().enumerate() {
                    if matches.len() >= max_results {
                        truncated = true;
                        break 'outer_fs;
                    }
                    let Some(col_byte) =
                        match_in_line(line_text, pattern, case_sensitive, whole_word)
                    else {
                        continue;
                    };
                    let column = line_text[..col_byte].chars().count() + 1;
                    matched_files.insert(rel_path.clone());
                    matches.push(TextSearchMatch {
                        file: rel_path.clone(),
                        line: offset + 1,
                        column,
                        text: line_text.to_string(),
                    });
                }
            }
        }

        let output = TextSearchOutput {
            pattern: pattern.to_string(),
            source,
            scanned_files,
            matched_files: matched_files.len(),
            skipped_large_files,
            returned: matches.len(),
            truncated,
            matches,
        };

        let mut outcome = CommandOutcome::from_value(output)?;
        outcome.meta.config_path = project_ctx.config_path;
        outcome.meta.profile = Some(project_ctx.profile_name.clone());
        outcome.meta.profile_path = project_ctx.profile_path.clone();
        outcome.meta.index_updated = Some(false);
        outcome.meta.warm = Some(warm.warmed);
        outcome.meta.warm_cost_ms = Some(warm.warm_cost_ms);
        outcome.meta.warm_graph_cache_hit = Some(warm.graph_cache_hit);
        outcome.hints.extend(project_ctx.hints);
        if !request_options.include_paths.is_empty() || !request_options.exclude_paths.is_empty() {
            outcome.hints.push(Hint {
                kind: HintKind::Info,
                text: format!(
                    "Path filters active: include=[{}] exclude=[{}]",
                    join_limited(&request_options.include_paths, 6),
                    join_limited(&request_options.exclude_paths, 6)
                ),
            });
        }
        if let Some(pat) = file_pattern {
            outcome.hints.push(Hint {
                kind: HintKind::Info,
                text: format!("File pattern: {pat}"),
            });
        }
        if truncated {
            let next_max_results = max_results.saturating_mul(2).min(1000);
            let mut args = Map::new();
            args.insert(
                "project".to_string(),
                Value::String(project_ctx.root.display().to_string()),
            );
            args.insert("pattern".to_string(), Value::String(pattern.to_string()));
            args.insert(
                "max_results".to_string(),
                Value::Number(serde_json::Number::from(next_max_results as u64)),
            );
            args.insert("case_sensitive".to_string(), Value::Bool(case_sensitive));
            args.insert("whole_word".to_string(), Value::Bool(whole_word));
            if let Some(pat) = file_pattern {
                args.insert("file_pattern".to_string(), Value::String(pat.to_string()));
            }
            outcome.next_actions.push(ToolNextAction {
                tool: "text_search".to_string(),
                args: Value::Object(args),
                reason: "Retry text_search with a higher max_results budget.".to_string(),
            });
        }
        Ok(outcome)
    }
}

async fn load_chunk_corpus(root: &Path) -> Result<Option<ChunkCorpus>> {
    let path = corpus_path_for_project_root(root);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(ChunkCorpus::load(&path).await?))
}

fn normalize_relative_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let rel = rel.to_string_lossy().into_owned();
    Some(rel.replace('\\', "/"))
}

fn find_word_boundary(haystack: &str, needle: &str) -> Option<usize> {
    let needle_is_ident = needle.bytes().all(is_ident_byte);
    if !needle_is_ident {
        return haystack.find(needle);
    }

    let bytes = haystack.as_bytes();
    for (idx, _) in haystack.match_indices(needle) {
        let left_ok = idx == 0 || !is_ident_byte(bytes[idx - 1]);
        let right_idx = idx + needle.len();
        let right_ok = right_idx >= bytes.len() || !is_ident_byte(bytes[right_idx]);
        if left_ok && right_ok {
            return Some(idx);
        }
    }
    None
}

const fn is_ident_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
}

fn match_in_line(
    line: &str,
    pattern: &str,
    case_sensitive: bool,
    whole_word: bool,
) -> Option<usize> {
    if case_sensitive {
        if whole_word {
            find_word_boundary(line, pattern)
        } else {
            line.find(pattern)
        }
    } else {
        let line_lower = line.to_ascii_lowercase();
        let pat_lower = pattern.to_ascii_lowercase();
        if whole_word {
            find_word_boundary(&line_lower, &pat_lower)
        } else {
            line_lower.find(&pat_lower)
        }
    }
}

fn join_limited(items: &[String], max: usize) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    if items.len() <= max {
        return items.join(", ");
    }
    format!(
        "{} …(+{})",
        items[..max].join(", "),
        items.len().saturating_sub(max)
    )
}
