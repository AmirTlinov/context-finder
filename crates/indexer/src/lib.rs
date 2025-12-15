//! # Context Indexer
//!
//! Project indexing for semantic code search.
//!
//! ## Pipeline
//!
//! ```text
//! Directory
//!     │
//!     ├──> File Scanner (.gitignore aware)
//!     │      └─> Source files
//!     │
//!     ├──> Chunker (AST-aware)
//!     │      └─> Code chunks
//!     │
//!     └──> Vector Store (batch embed)
//!            └─> Searchable index
//! ```
//!
//! ## Example
//!
//! ```no_run
//! use context_indexer::ProjectIndexer;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let indexer = ProjectIndexer::new("/path/to/project").await?;
//!     let stats = indexer.index().await?;
//!
//!     println!("Indexed {} files, {} chunks", stats.files, stats.chunks);
//!     Ok(())
//! }
//! ```

mod error;
mod health;
mod indexer;
mod scanner;
mod stats;
mod watcher;

pub use error::{IndexerError, Result};
pub use health::append_failure_reason;
pub use health::{health_file_path, read_health_snapshot, write_health_snapshot, HealthSnapshot};
pub use indexer::{ModelIndexSpec, MultiModelProjectIndexer, ProjectIndexer};
pub use scanner::FileScanner;
pub use stats::IndexStats;
pub use watcher::{
    IndexUpdate, IndexerHealth, MultiModelStreamingIndexer, StreamingIndexer,
    StreamingIndexerConfig,
};
