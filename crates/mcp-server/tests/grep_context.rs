use anyhow::{Context, Result};
use rmcp::{model::CallToolRequestParam, service::ServiceExt, transport::TokioChildProcess};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

fn locate_context_finder_mcp_bin() -> Result<PathBuf> {
    if let Some(path) = option_env!("CARGO_BIN_EXE_context-finder-mcp") {
        return Ok(PathBuf::from(path));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(target_profile_dir) = exe.parent().and_then(|p| p.parent()) {
            let candidate = target_profile_dir.join("context-finder-mcp");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .ancestors()
        .nth(2)
        .context("failed to resolve repo root from CARGO_MANIFEST_DIR")?;
    for rel in [
        "target/debug/context-finder-mcp",
        "target/release/context-finder-mcp",
    ] {
        let candidate = repo_root.join(rel);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    anyhow::bail!("failed to locate context-finder-mcp binary")
}

#[tokio::test]
async fn grep_context_works_without_index_and_merges_ranges() -> Result<()> {
    let bin = locate_context_finder_mcp_bin()?;

    let mut cmd = Command::new(bin);
    cmd.env_remove("CONTEXT_FINDER_MODEL_DIR");
    cmd.env("CONTEXT_FINDER_PROFILE", "quality");
    cmd.env("RUST_LOG", "warn");
    cmd.env("CONTEXT_FINDER_DISABLE_DAEMON", "1");

    let transport = TokioChildProcess::new(cmd).context("spawn mcp server")?;
    let service = tokio::time::timeout(Duration::from_secs(10), ().serve(transport))
        .await
        .context("timeout starting MCP server")??;

    let tmp = tempfile::tempdir().context("tempdir")?;
    let root = tmp.path();
    std::fs::create_dir_all(root.join("src")).context("mkdir src")?;

    let mut lines = Vec::new();
    for i in 1..=12usize {
        if i == 5 || i == 7 {
            lines.push(format!("line {i}: TARGET"));
        } else {
            lines.push(format!("line {i}: filler"));
        }
    }
    std::fs::write(root.join("src").join("a.txt"), lines.join("\n") + "\n")
        .context("write a.txt")?;
    std::fs::write(
        root.join("src").join("b.txt"),
        "one\nTwo\nthree TARGET\nfour\n",
    )
    .context("write b.txt")?;

    assert!(
        !root.join(".context-finder").exists(),
        "temp project unexpectedly has .context-finder before grep_context"
    );

    let args = serde_json::json!({
        "path": root.to_string_lossy(),
        "pattern": "TARGET",
        "file_pattern": "src/*",
        "before": 2,
        "after": 2,
        "max_matches": 100,
        "max_hunks": 10,
        "max_chars": 20_000,
        "case_sensitive": true,
    });
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        service.call_tool(CallToolRequestParam {
            name: "grep_context".into(),
            arguments: args.as_object().cloned(),
        }),
    )
    .await
    .context("timeout calling grep_context")??;

    assert_ne!(result.is_error, Some(true), "grep_context returned error");
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .context("grep_context did not return text content")?;
    let json: Value =
        serde_json::from_str(text).context("grep_context output is not valid JSON")?;

    assert_eq!(json.get("pattern").and_then(Value::as_str), Some("TARGET"));
    assert_eq!(json.get("truncated").and_then(Value::as_bool), Some(false));

    let hunks = json
        .get("hunks")
        .and_then(Value::as_array)
        .context("missing hunks array")?;
    assert!(hunks.len() >= 2, "expected at least two hunks");

    let a_hunk = hunks
        .iter()
        .find(|h| h.get("file").and_then(Value::as_str) == Some("src/a.txt"))
        .context("missing hunk for src/a.txt")?;
    assert_eq!(a_hunk.get("start_line").and_then(Value::as_u64), Some(3));

    let match_lines: Vec<u64> = a_hunk
        .get("match_lines")
        .and_then(Value::as_array)
        .context("src/a.txt missing match_lines")?
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    assert_eq!(match_lines, vec![5, 7]);

    let end_line = a_hunk
        .get("end_line")
        .and_then(Value::as_u64)
        .context("src/a.txt missing end_line")?;
    assert!(end_line >= 9, "expected merged range to include line 9");

    let content = a_hunk
        .get("content")
        .and_then(Value::as_str)
        .context("src/a.txt missing content")?;
    assert!(content.contains("line 5: TARGET"));
    assert!(content.contains("line 7: TARGET"));

    assert!(
        !root.join(".context-finder").exists(),
        "grep_context created .context-finder side effects"
    );

    service.cancel().await.context("shutdown mcp service")?;
    Ok(())
}

#[tokio::test]
async fn grep_context_can_be_case_insensitive_and_reports_max_chars_truncation() -> Result<()> {
    let bin = locate_context_finder_mcp_bin()?;

    let mut cmd = Command::new(bin);
    cmd.env_remove("CONTEXT_FINDER_MODEL_DIR");
    cmd.env("CONTEXT_FINDER_PROFILE", "quality");
    cmd.env("RUST_LOG", "warn");
    cmd.env("CONTEXT_FINDER_DISABLE_DAEMON", "1");

    let transport = TokioChildProcess::new(cmd).context("spawn mcp server")?;
    let service = tokio::time::timeout(Duration::from_secs(10), ().serve(transport))
        .await
        .context("timeout starting MCP server")??;

    let tmp = tempfile::tempdir().context("tempdir")?;
    let root = tmp.path();
    std::fs::create_dir_all(root.join("src")).context("mkdir src")?;
    std::fs::write(
        root.join("src").join("main.txt"),
        "aaa\nTARGETTARGETTARGETTARGET\ncccccccccccccccccccc\n",
    )
    .context("write main.txt")?;

    assert!(
        !root.join(".context-finder").exists(),
        "temp project unexpectedly has .context-finder before grep_context"
    );

    let args = serde_json::json!({
        "path": root.to_string_lossy(),
        "pattern": "target",
        "file": "src/main.txt",
        "before": 1,
        "after": 1,
        "max_matches": 10,
        "max_hunks": 10,
        "max_chars": 35,
        "case_sensitive": false,
    });
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        service.call_tool(CallToolRequestParam {
            name: "grep_context".into(),
            arguments: args.as_object().cloned(),
        }),
    )
    .await
    .context("timeout calling grep_context")??;

    assert_ne!(result.is_error, Some(true), "grep_context returned error");
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .context("grep_context did not return text content")?;
    let json: Value =
        serde_json::from_str(text).context("grep_context output is not valid JSON")?;

    assert_eq!(
        json.get("case_sensitive").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(json.get("truncated").and_then(Value::as_bool), Some(true));
    assert_eq!(
        json.get("truncation").and_then(Value::as_str),
        Some("max_chars")
    );

    let hunks = json
        .get("hunks")
        .and_then(Value::as_array)
        .context("missing hunks array")?;
    assert_eq!(hunks.len(), 1);
    let hunk = hunks.first().context("expected a single hunk")?;
    assert_eq!(
        hunk.get("file").and_then(Value::as_str),
        Some("src/main.txt")
    );

    let content = hunk
        .get("content")
        .and_then(Value::as_str)
        .context("missing content")?;
    assert!(
        content.contains("TARGETTARGETTARGETTARGET"),
        "expected match line in content, got: {content:?}"
    );
    assert!(
        !content.contains("cccccccccccccccccccc"),
        "expected last context line to be truncated"
    );

    assert!(
        !root.join(".context-finder").exists(),
        "grep_context created .context-finder side effects"
    );

    service.cancel().await.context("shutdown mcp service")?;
    Ok(())
}
