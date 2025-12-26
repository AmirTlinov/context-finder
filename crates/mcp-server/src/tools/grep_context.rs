use anyhow::{Context as AnyhowContext, Result};
use context_indexer::FileScanner;
use regex::Regex;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::cursor::{encode_cursor, CURSOR_VERSION};
use super::paths::normalize_relative_path;
use super::schemas::grep_context::{
    GrepContextCursorV1, GrepContextHunk, GrepContextRequest, GrepContextResult,
    GrepContextTruncation,
};
use super::ContextFinderService;

#[derive(Debug, Clone)]
struct GrepRange {
    start_line: usize,
    end_line: usize,
    match_lines: Vec<usize>,
}

fn merge_grep_ranges(mut ranges: Vec<GrepRange>) -> Vec<GrepRange> {
    ranges.sort_by(|a, b| {
        a.start_line
            .cmp(&b.start_line)
            .then_with(|| a.end_line.cmp(&b.end_line))
    });

    let mut merged: Vec<GrepRange> = Vec::new();
    for range in ranges {
        let Some(last) = merged.last_mut() else {
            merged.push(range);
            continue;
        };

        if range.start_line <= last.end_line.saturating_add(1) {
            last.end_line = last.end_line.max(range.end_line);
            last.match_lines.extend(range.match_lines);
            continue;
        }

        merged.push(range);
    }

    for range in &mut merged {
        range.match_lines.sort_unstable();
        range.match_lines.dedup();
    }

    merged
}

pub(super) struct GrepContextComputeOptions<'a> {
    pub(super) case_sensitive: bool,
    pub(super) before: usize,
    pub(super) after: usize,
    pub(super) max_matches: usize,
    pub(super) max_hunks: usize,
    pub(super) max_chars: usize,
    pub(super) resume_file: Option<&'a str>,
    pub(super) resume_line: usize,
}

#[derive(Debug)]
struct MatchScanResult {
    match_lines: Vec<usize>,
    hit_match_limit: bool,
}

#[derive(Debug)]
struct GrepContextAccumulators {
    hunks: Vec<GrepContextHunk>,
    used_chars: usize,
    truncated: bool,
    truncation: Option<GrepContextTruncation>,
    scanned_files: usize,
    matched_files: usize,
    returned_matches: usize,
    total_matches: usize,
    next_cursor_state: Option<(String, usize)>,
}

impl GrepContextAccumulators {
    const fn new() -> Self {
        Self {
            hunks: Vec::new(),
            used_chars: 0,
            truncated: false,
            truncation: None,
            scanned_files: 0,
            matched_files: 0,
            returned_matches: 0,
            total_matches: 0,
            next_cursor_state: None,
        }
    }
}

fn canonicalize_request_file(root: &Path, file: &str) -> Result<(String, PathBuf)> {
    let canonical = root
        .join(Path::new(file))
        .canonicalize()
        .with_context(|| format!("Invalid file '{file}'"))?;
    if !canonical.starts_with(root) {
        anyhow::bail!("File '{file}' is outside project root");
    }

    let display = normalize_relative_path(root, &canonical)
        .unwrap_or_else(|| canonical.to_string_lossy().into_owned().replace('\\', "/"));
    Ok((display, canonical))
}

async fn collect_candidates(
    root: &Path,
    request: &GrepContextRequest,
    file_pattern: Option<&str>,
) -> Result<(String, Vec<(String, PathBuf)>)> {
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();

    if let Some(file) = request
        .file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let (display, canonical) = canonicalize_request_file(root, file)?;
        candidates.push((display, canonical));
        return Ok(("filesystem".to_string(), candidates));
    }

    if let Some(corpus) = ContextFinderService::load_chunk_corpus(root).await? {
        let mut files: Vec<&String> = corpus.files().keys().collect();
        files.sort();
        for file in files {
            if !ContextFinderService::matches_file_pattern(file, file_pattern) {
                continue;
            }
            candidates.push((file.clone(), root.join(file)));
        }
        return Ok(("corpus".to_string(), candidates));
    }

    let scanner = FileScanner::new(root);
    let files = scanner.scan();
    let mut rels: Vec<String> = files
        .into_iter()
        .filter_map(|p| normalize_relative_path(root, &p))
        .collect();
    rels.sort();
    for rel in rels {
        if !ContextFinderService::matches_file_pattern(&rel, file_pattern) {
            continue;
        }
        candidates.push((rel.clone(), root.join(&rel)));
    }

    Ok(("filesystem".to_string(), candidates))
}

fn ensure_resume_file_exists(
    resume_file: Option<&str>,
    candidates: &[(String, PathBuf)],
) -> Result<()> {
    let Some(resume_file) = resume_file else {
        return Ok(());
    };

    if candidates.iter().any(|(file, _)| file == resume_file) {
        Ok(())
    } else {
        anyhow::bail!("Cursor resume_file not found: {resume_file}");
    }
}

fn trimmed_non_empty_str(input: Option<&str>) -> Option<&str> {
    input.map(str::trim).filter(|value| !value.is_empty())
}

fn file_resume_line(display_file: &str, resume_file: Option<&str>, resume_line: usize) -> usize {
    if Some(display_file) == resume_file {
        resume_line
    } else {
        1
    }
}

fn scan_match_lines_for_file(
    file_path: &Path,
    regex: &Regex,
    file_resume_line: usize,
    max_matches: usize,
    total_matches: &mut usize,
) -> std::result::Result<MatchScanResult, std::io::Error> {
    let file = std::fs::File::open(file_path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_no = 0usize;
    let mut match_lines: Vec<usize> = Vec::new();
    let mut hit_match_limit = false;

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }
        line_no += 1;

        let text = line.trim_end_matches(&['\r', '\n'][..]);
        if !regex.is_match(text) {
            continue;
        }
        match_lines.push(line_no);
        if line_no >= file_resume_line {
            *total_matches += 1;
            if *total_matches >= max_matches {
                hit_match_limit = true;
                break;
            }
        }
    }

    Ok(MatchScanResult {
        match_lines,
        hit_match_limit,
    })
}

fn build_ranges_from_matches(match_lines: &[usize], before: usize, after: usize) -> Vec<GrepRange> {
    let ranges: Vec<GrepRange> = match_lines
        .iter()
        .map(|&ln| {
            let start_line = ln.saturating_sub(before).max(1);
            let end_line = ln.saturating_add(after);
            GrepRange {
                start_line,
                end_line,
                match_lines: vec![ln],
            }
        })
        .collect();

    merge_grep_ranges(ranges)
}

fn build_hunks_for_file(
    acc: &mut GrepContextAccumulators,
    display_file: String,
    file_path: &Path,
    file_resume_line: usize,
    ranges: &[GrepRange],
    max_hunks: usize,
    max_chars: usize,
) -> bool {
    let Ok(file) = std::fs::File::open(file_path) else {
        return true;
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_no = 0usize;
    let mut range_idx = 0usize;

    while range_idx < ranges.len() {
        let range = &ranges[range_idx];
        let range_start_line = range.start_line.max(file_resume_line);
        if range_start_line > range.end_line {
            range_idx += 1;
            continue;
        }

        if acc.hunks.len() >= max_hunks {
            acc.truncated = true;
            acc.truncation = Some(GrepContextTruncation::Hunks);
            acc.next_cursor_state = Some((display_file, range_start_line));
            return false;
        }

        let mut content = String::new();
        let mut end_line = range_start_line.saturating_sub(1);
        let mut stop_due_to_budget = false;

        loop {
            line.clear();
            let Ok(bytes_read) = reader.read_line(&mut line) else {
                break;
            };
            if bytes_read == 0 {
                break;
            }
            line_no += 1;

            if line_no < range_start_line {
                continue;
            }
            if line_no > range.end_line {
                break;
            }

            let text = line.trim_end_matches(&['\r', '\n'][..]);
            let line_chars = text.chars().count();
            let extra_chars = if content.is_empty() {
                line_chars
            } else {
                1 + line_chars
            };

            if acc.used_chars.saturating_add(extra_chars) > max_chars {
                acc.truncated = true;
                acc.truncation = Some(GrepContextTruncation::Chars);
                stop_due_to_budget = true;
                break;
            }

            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str(text);
            acc.used_chars += extra_chars;
            end_line = line_no;
        }

        if stop_due_to_budget && content.is_empty() {
            return false;
        }

        let mut match_lines = ranges[range_idx].match_lines.clone();
        match_lines.retain(|&ln| ln >= range_start_line && ln <= end_line);
        acc.returned_matches += match_lines.len();

        acc.hunks.push(GrepContextHunk {
            file: display_file.clone(),
            start_line: range_start_line,
            end_line,
            match_lines,
            content,
        });

        if stop_due_to_budget {
            acc.next_cursor_state = Some((display_file, end_line.saturating_add(1)));
            return false;
        }

        range_idx += 1;
    }

    true
}

fn build_next_cursor(
    root_display: &str,
    request: &GrepContextRequest,
    file_pattern: Option<&str>,
    case_sensitive: bool,
    before: usize,
    after: usize,
    cursor_state: Option<(String, usize)>,
) -> Result<Option<String>> {
    let Some((resume_file, resume_line)) = cursor_state else {
        return Ok(None);
    };

    let token = GrepContextCursorV1 {
        v: CURSOR_VERSION,
        tool: "grep_context".to_string(),
        root: root_display.to_string(),
        pattern: request.pattern.clone(),
        file: request
            .file
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        file_pattern: file_pattern.map(str::to_string),
        case_sensitive,
        before,
        after,
        resume_file,
        resume_line,
    };

    Ok(Some(encode_cursor(&token)?))
}

pub(super) async fn compute_grep_context_result(
    root: &Path,
    root_display: &str,
    request: &GrepContextRequest,
    regex: &Regex,
    opts: GrepContextComputeOptions<'_>,
) -> Result<GrepContextResult> {
    const MAX_FILE_BYTES: u64 = 2_000_000;
    let GrepContextComputeOptions {
        case_sensitive,
        before,
        after,
        max_matches,
        max_hunks,
        max_chars,
        resume_file,
        resume_line,
    } = opts;

    let file_pattern = trimmed_non_empty_str(request.file_pattern.as_deref());
    let resume_file = trimmed_non_empty_str(resume_file);
    let resume_line = resume_line.max(1);
    let (source, candidates) = collect_candidates(root, request, file_pattern).await?;
    ensure_resume_file_exists(resume_file, &candidates)?;

    let mut acc = GrepContextAccumulators::new();
    let mut started = resume_file.is_none();
    'outer_files: for (display_file, file_path) in candidates {
        if !started {
            if Some(display_file.as_str()) != resume_file {
                continue;
            }
            started = true;
        }

        let file_resume_line = file_resume_line(display_file.as_str(), resume_file, resume_line);

        acc.scanned_files += 1;

        let Ok(meta) = std::fs::metadata(&file_path) else {
            continue;
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }

        let Ok(scan) = scan_match_lines_for_file(
            &file_path,
            regex,
            file_resume_line,
            max_matches,
            &mut acc.total_matches,
        ) else {
            continue;
        };

        if scan.match_lines.is_empty() {
            continue;
        }
        acc.matched_files += 1;
        if scan.hit_match_limit {
            acc.truncated = true;
            acc.truncation = Some(GrepContextTruncation::Matches);
        }

        let ranges = build_ranges_from_matches(&scan.match_lines, before, after);

        if !build_hunks_for_file(
            &mut acc,
            display_file,
            &file_path,
            file_resume_line,
            &ranges,
            max_hunks,
            max_chars,
        ) {
            break 'outer_files;
        }

        if scan.hit_match_limit {
            break 'outer_files;
        }
    }

    let next_cursor = build_next_cursor(
        root_display,
        request,
        file_pattern,
        case_sensitive,
        before,
        after,
        acc.next_cursor_state.take(),
    )?;

    let result = GrepContextResult {
        pattern: request.pattern.clone(),
        source,
        file: request.file.clone(),
        file_pattern: request.file_pattern.clone(),
        case_sensitive,
        before,
        after,
        scanned_files: acc.scanned_files,
        matched_files: acc.matched_files,
        returned_matches: acc.returned_matches,
        returned_hunks: acc.hunks.len(),
        used_chars: acc.used_chars,
        max_chars,
        truncated: acc.truncated,
        truncation: acc.truncation,
        next_cursor,
        next_actions: None,
        meta: None,
        hunks: acc.hunks,
    };

    Ok(result)
}
