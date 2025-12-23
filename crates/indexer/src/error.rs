use thiserror::Error;

pub type Result<T> = std::result::Result<T, IndexerError>;

#[derive(Error, Debug)]
pub enum IndexerError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Chunker error: {0}")]
    ChunkerError(#[from] context_code_chunker::ChunkerError),

    #[error("Vector store error: {0}")]
    VectorStoreError(#[from] context_vector_store::VectorStoreError),

    #[error("Invalid project path: {0}")]
    InvalidPath(String),

    #[error("System time error: {0}")]
    SystemTimeError(#[from] std::time::SystemTimeError),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Index budget exceeded")]
    BudgetExceeded,

    #[error("{0}")]
    Other(String),
}
