use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const INDEX_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Watermark {
    Git {
        #[serde(skip_serializing_if = "Option::is_none")]
        computed_at_unix_ms: Option<u64>,
        git_head: String,
        git_dirty: bool,
    },
    Filesystem {
        #[serde(skip_serializing_if = "Option::is_none")]
        computed_at_unix_ms: Option<u64>,
        file_count: u64,
        max_mtime_ms: u64,
        total_bytes: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StaleReason {
    IndexMissing,
    IndexCorrupt,
    WatermarkMissing,
    GitHeadMismatch,
    GitDirtyMismatch,
    FilesystemChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReindexResult {
    Ok,
    BudgetExceeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ReindexAttempt {
    pub attempted: bool,
    pub performed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ReindexResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct IndexSnapshot {
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub built_at_unix_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watermark: Option<Watermark>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct IndexState {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    pub model_id: String,
    pub profile: String,
    pub project_watermark: Watermark,
    pub index: IndexSnapshot,
    pub stale: bool,
    #[serde(default)]
    pub stale_reasons: Vec<StaleReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reindex: Option<ReindexAttempt>,
}

#[derive(Debug, Clone, PartialEq, Eq, JsonSchema)]
pub struct StaleAssessment {
    pub stale: bool,
    pub reasons: Vec<StaleReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ToolMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_state: Option<IndexState>,
}

#[must_use]
pub fn assess_staleness(
    project_watermark: &Watermark,
    index_exists: bool,
    index_corrupt: bool,
    index_watermark: Option<&Watermark>,
) -> StaleAssessment {
    let mut reasons = Vec::new();

    if !index_exists {
        reasons.push(StaleReason::IndexMissing);
    }
    if index_corrupt {
        reasons.push(StaleReason::IndexCorrupt);
    }

    match index_watermark {
        None => {
            if index_exists {
                reasons.push(StaleReason::WatermarkMissing);
            }
        }
        Some(index_mark) => match (index_mark, project_watermark) {
            (
                Watermark::Git {
                    git_head: idx_head,
                    git_dirty: idx_dirty,
                    ..
                },
                Watermark::Git {
                    git_head: cur_head,
                    git_dirty: cur_dirty,
                    ..
                },
            ) => {
                if idx_head != cur_head {
                    reasons.push(StaleReason::GitHeadMismatch);
                }
                if idx_dirty != cur_dirty {
                    reasons.push(StaleReason::GitDirtyMismatch);
                }
            }
            (
                Watermark::Filesystem {
                    file_count: idx_files,
                    max_mtime_ms: idx_mtime,
                    total_bytes: idx_bytes,
                    ..
                },
                Watermark::Filesystem {
                    file_count: cur_files,
                    max_mtime_ms: cur_mtime,
                    total_bytes: cur_bytes,
                    ..
                },
            ) => {
                if idx_files != cur_files || idx_mtime != cur_mtime || idx_bytes != cur_bytes {
                    reasons.push(StaleReason::FilesystemChanged);
                }
            }
            _ => reasons.push(StaleReason::FilesystemChanged),
        },
    }

    let stale = !reasons.is_empty();
    StaleAssessment { stale, reasons }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn git(head: &str, dirty: bool) -> Watermark {
        Watermark::Git {
            computed_at_unix_ms: None,
            git_head: head.to_string(),
            git_dirty: dirty,
        }
    }

    fn fs(files: u64, max_mtime_ms: u64, bytes: u64) -> Watermark {
        Watermark::Filesystem {
            computed_at_unix_ms: None,
            file_count: files,
            max_mtime_ms,
            total_bytes: bytes,
        }
    }

    #[test]
    fn stale_when_index_missing() {
        let out = assess_staleness(&git("abc", false), false, false, None);
        assert_eq!(out.stale, true);
        assert_eq!(out.reasons, vec![StaleReason::IndexMissing]);
    }

    #[test]
    fn stale_when_index_corrupt() {
        let out = assess_staleness(&git("abc", false), true, true, Some(&git("abc", false)));
        assert_eq!(out.stale, true);
        assert_eq!(out.reasons, vec![StaleReason::IndexCorrupt]);
    }

    #[test]
    fn stale_when_watermark_missing() {
        let out = assess_staleness(&git("abc", false), true, false, None);
        assert_eq!(out.stale, true);
        assert_eq!(out.reasons, vec![StaleReason::WatermarkMissing]);
    }

    #[test]
    fn stale_when_git_head_mismatch() {
        let out = assess_staleness(&git("bbb", false), true, false, Some(&git("aaa", false)));
        assert_eq!(out.stale, true);
        assert_eq!(out.reasons, vec![StaleReason::GitHeadMismatch]);
    }

    #[test]
    fn stale_when_git_dirty_mismatch() {
        let out = assess_staleness(&git("aaa", true), true, false, Some(&git("aaa", false)));
        assert_eq!(out.stale, true);
        assert_eq!(out.reasons, vec![StaleReason::GitDirtyMismatch]);
    }

    #[test]
    fn stale_when_filesystem_changed() {
        let out = assess_staleness(&fs(10, 123, 50), true, false, Some(&fs(10, 124, 50)));
        assert_eq!(out.stale, true);
        assert_eq!(out.reasons, vec![StaleReason::FilesystemChanged]);
    }

    #[test]
    fn fresh_when_git_equal() {
        let out = assess_staleness(&git("aaa", false), true, false, Some(&git("aaa", false)));
        assert_eq!(out.stale, false);
        assert_eq!(out.reasons, Vec::<StaleReason>::new());
    }

    #[test]
    fn fresh_when_filesystem_equal() {
        let mark = fs(10, 123, 50);
        let out = assess_staleness(&mark, true, false, Some(&mark));
        assert_eq!(out.stale, false);
        assert_eq!(out.reasons, Vec::<StaleReason>::new());
    }
}
