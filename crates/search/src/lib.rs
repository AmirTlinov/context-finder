mod context_search;
mod error;
mod fusion;
mod fuzzy;
mod rerank;
pub mod hybrid;
pub mod profile;
pub use context_vector_store::SearchResult;
mod query_classifier;
mod query_expansion;

pub use context_search::{ContextSearch, EnrichedResult, RelatedContext};
pub use error::{Result, SearchError};
pub use fusion::{AstBooster, RRFFusion};
pub use fuzzy::FuzzySearch;
pub use hybrid::HybridSearch;
pub use profile::{Bm25Config, MatchKind, RerankConfig, SearchProfile, Thresholds};
pub use query_classifier::{QueryClassifier, QueryType, QueryWeights};
pub use query_expansion::QueryExpander;
