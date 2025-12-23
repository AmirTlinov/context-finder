use crate::scanner::FileScanner;
use crate::{IndexerError, Result, Watermark};
use serde::{Deserialize, Serialize};
use std::cmp::max;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const INDEX_WATERMARK_FILE_NAME: &str = "watermark.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedIndexWatermark {
    pub built_at_unix_ms: u64,
    pub watermark: Watermark,
}

pub fn index_watermark_path_for_store(store_path: &Path) -> Result<PathBuf> {
    let dir = store_path
        .parent()
        .ok_or_else(|| IndexerError::InvalidPath("store path has no parent".into()))?;
    Ok(dir.join(INDEX_WATERMARK_FILE_NAME))
}

pub async fn write_index_watermark(store_path: &Path, watermark: Watermark) -> Result<()> {
    let path = index_watermark_path_for_store(store_path)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let built_at_unix_ms = unix_now_ms();
    let persisted = PersistedIndexWatermark {
        built_at_unix_ms,
        watermark,
    };

    let bytes = serde_json::to_vec_pretty(&persisted)?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

pub async fn read_index_watermark(store_path: &Path) -> Result<Option<PersistedIndexWatermark>> {
    let path = index_watermark_path_for_store(store_path)?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = tokio::fs::read(&path).await?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub async fn compute_project_watermark(project_root: &Path) -> Result<Watermark> {
    if let Some(mark) = try_compute_git_watermark(project_root).await {
        return Ok(mark);
    }
    compute_filesystem_watermark(project_root).await
}

async fn try_compute_git_watermark(project_root: &Path) -> Option<Watermark> {
    let head = tokio::process::Command::new("git")
        .arg("-C")
        .arg(project_root)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .await
        .ok()?;
    if !head.status.success() {
        return None;
    }
    let git_head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    if git_head.is_empty() {
        return None;
    }

    let status = tokio::process::Command::new("git")
        .arg("-C")
        .arg(project_root)
        .arg("status")
        .arg("--porcelain")
        .output()
        .await
        .ok()?;
    if !status.status.success() {
        return None;
    }
    let git_dirty = !status.stdout.is_empty();

    Some(Watermark::Git {
        computed_at_unix_ms: Some(unix_now_ms()),
        git_head,
        git_dirty,
    })
}

async fn compute_filesystem_watermark(project_root: &Path) -> Result<Watermark> {
    let root = project_root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let scanner = FileScanner::new(&root);
        let files = scanner.scan();

        let mut file_count = 0u64;
        let mut total_bytes = 0u64;
        let mut max_mtime_ms = 0u64;

        for path in files {
            let meta = std::fs::metadata(&path)?;
            file_count += 1;
            total_bytes = total_bytes.saturating_add(meta.len());
            if let Ok(modified) = meta.modified() {
                let mtime_ms = modified
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                max_mtime_ms = max(max_mtime_ms, mtime_ms);
            }
        }

        Ok::<_, IndexerError>(Watermark::Filesystem {
            computed_at_unix_ms: Some(unix_now_ms()),
            file_count,
            max_mtime_ms,
            total_bytes,
        })
    })
    .await
    .map_err(|e| IndexerError::Other(format!("failed to compute filesystem watermark: {e}")))?
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
