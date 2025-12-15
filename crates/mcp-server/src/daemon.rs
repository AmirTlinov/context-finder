use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const TTL_MS: u64 = 5 * 60 * 1000;

#[derive(Serialize, Deserialize)]
struct PingRequest {
    cmd: String,
    project: String,
    #[serde(default)]
    ttl_ms: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct PingResponse {
    status: String,
    message: Option<String>,
}

fn default_socket_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".context-finder")
        .join("daemon.sock")
}

pub async fn touch(project: &Path) -> Result<()> {
    let socket = default_socket_path();
    ensure_daemon(&socket).await?;
    let payload = PingRequest {
        cmd: "ping".to_string(),
        project: project.to_string_lossy().to_string(),
        ttl_ms: Some(TTL_MS),
    };
    if send_ping(&socket, &payload).await.is_ok() {
        return Ok(());
    }
    // maybe daemon died, restart once
    ensure_daemon(&socket).await?;
    send_ping(&socket, &payload).await?;
    Ok(())
}

async fn send_ping(socket: &Path, payload: &PingRequest) -> Result<()> {
    let mut stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connect to daemon at {}", socket.display()))?;
    let msg = serde_json::to_string(payload)? + "\n";
    stream.write_all(msg.as_bytes()).await?;
    stream.flush().await?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let resp: PingResponse = serde_json::from_str(&line)?;
    if resp.status == "ok" {
        Ok(())
    } else {
        anyhow::bail!(resp.message.unwrap_or_else(|| "daemon error".to_string()))
    }
}

async fn ensure_daemon(socket: &Path) -> Result<()> {
    if UnixStream::connect(socket).await.is_ok() {
        return Ok(());
    }

    let exe = resolve_daemon_exe()?;
    tokio::process::Command::new(exe)
        .arg("daemon-loop")
        .arg("--socket")
        .arg(socket)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| "failed to spawn daemon-loop")?;

    let mut retries = 0;
    while retries < 20 {
        if UnixStream::connect(socket).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        retries += 1;
    }
    anyhow::bail!("daemon did not start in time")
}

fn resolve_daemon_exe() -> Result<PathBuf> {
    if let Ok(raw) = std::env::var("CONTEXT_FINDER_DAEMON_EXE") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let exe = std::env::current_exe()?;
    if let Some(dir) = exe.parent() {
        let candidate = dir.join("context-finder");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Ok(PathBuf::from("context-finder"))
}
