use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("index not found in S3")]
    IndexNotFound,

    #[error("index corrupted: {0}")]
    IndexCorrupted(String),

    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),

    #[error("query parse error: {0}")]
    QueryParse(String),

    #[error("storage error: {0}")]
    Storage(#[from] claria_storage::error::StorageError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ETag mismatch: index was modified by another writer")]
    ETagMismatch,

    #[error("document not found: {0}")]
    DocumentNotFound(String),
}
