use crate::cache::{compare_cache_key, load_compare, save_compare, CacheConfig};
use crate::command::domain::ComparisonOutput;
use anyhow::Result;
use std::path::Path;

#[derive(Clone)]
pub struct CompareCacheAdapter {
    cfg: CacheConfig,
}

impl CompareCacheAdapter {
    pub fn new(cfg: CacheConfig) -> Self {
        Self { cfg }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn key(
        &self,
        project: &Path,
        queries: &[String],
        limit: usize,
        strategy: &str,
        reuse_graph: bool,
        show_graph: bool,
        language: &str,
        index_mtime_ms: u64,
    ) -> String {
        compare_cache_key(
            project,
            queries,
            limit,
            strategy,
            reuse_graph,
            show_graph,
            language,
            index_mtime_ms,
        )
    }

    pub async fn load(&self, key: &str, store_mtime_ms: u64) -> Result<Option<ComparisonOutput>> {
        load_compare(&self.cfg, key, store_mtime_ms).await
    }

    pub async fn save(
        &self,
        key: &str,
        store_mtime_ms: u64,
        data: &ComparisonOutput,
    ) -> Result<()> {
        save_compare(&self.cfg, key, store_mtime_ms, data).await
    }
}
