use crate::graph_cache::GraphCache;
use std::path::Path;

#[derive(Clone, Default)]
pub struct GraphCacheFactory;

impl GraphCacheFactory {
    pub fn for_root(&self, root: &Path) -> GraphCache {
        GraphCache::new(root)
    }
}
