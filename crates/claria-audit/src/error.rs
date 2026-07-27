use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("CloudTrail error: {0}")]
    CloudTrail(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("audit store error: {0}")]
    Storage(#[from] claria_storage::error::StorageError),

    #[error("AWS config error: {0}")]
    Config(String),
}
