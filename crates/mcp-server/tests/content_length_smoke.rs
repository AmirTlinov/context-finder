use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

fn locate_context_finder_mcp_bin() -> Result<PathBuf> {
    if let Some(path) = option_env!("CARGO_BIN_EXE_context-finder-mcp") {
        return Ok(PathBuf::from(path));
    }

    // `.../target/{debug|release}/deps/<test>` → `.../target/{debug|release}/context-finder-mcp`
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

async fn send_frame(stdin: &mut tokio::process::ChildStdin, value: &Value) -> Result<()> {
    let json = serde_json::to_vec(value)?;
    let header = format!("Content-Length: {}\r\n\r\n", json.len());
    stdin.write_all(header.as_bytes()).await?;
    stdin.write_all(&json).await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_frame(stdout: &mut BufReader<tokio::process::ChildStdout>) -> Result<Value> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = stdout.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!("EOF while reading MCP frame headers");
        }
        if line == "\n" || line == "\r\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = Some(rest.trim().parse::<usize>()?);
        }
    }
    let len = content_length.context("missing Content-Length header")?;

    let mut body = vec![0u8; len];
    stdout.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

#[tokio::test]
async fn mcp_supports_content_length_framing() -> Result<()> {
    let bin = locate_context_finder_mcp_bin()?;

    let mut cmd = Command::new(bin);
    cmd.env("CONTEXT_FINDER_PROFILE", "quality");
    cmd.env("CONTEXT_FINDER_EMBEDDING_MODE", "stub");
    cmd.env_remove("CONTEXT_FINDER_DAEMON_EXE");
    cmd.env("RUST_LOG", "warn");
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());

    let mut child = cmd.spawn().context("spawn mcp server")?;
    let mut stdin = child.stdin.take().context("stdin")?;
    let stdout = child.stdout.take().context("stdout")?;
    let mut stdout = BufReader::new(stdout);

    // initialize
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "content-length-smoke", "version": "0.1" }
        }
    });
    send_frame(&mut stdin, &init_req).await?;
    let init_resp = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut stdout))
        .await
        .context("timeout reading initialize response")??;
    assert_eq!(init_resp.get("id").and_then(Value::as_i64), Some(1));

    // initialized notification
    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    send_frame(&mut stdin, &initialized).await?;

    // tools/list
    let list_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    send_frame(&mut stdin, &list_req).await?;
    let list_resp = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut stdout))
        .await
        .context("timeout reading tools/list response")??;
    assert_eq!(list_resp.get("id").and_then(Value::as_i64), Some(2));
    let tools = list_resp
        .get("result")
        .and_then(|v| v.get("tools"))
        .and_then(Value::as_array)
        .context("missing result.tools")?;
    assert!(
        tools
            .iter()
            .any(|t| t.get("name").and_then(Value::as_str) == Some("map")),
        "tools/list missing 'map'"
    );
    assert!(
        tools
            .iter()
            .any(|t| t.get("name").and_then(Value::as_str) == Some("text_search")),
        "tools/list missing 'text_search'"
    );

    // shutdown
    let _ = child.kill().await;
    Ok(())
}
