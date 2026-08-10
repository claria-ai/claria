use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("audit store error: {0}")]
    Storage(#[from] claria_storage::error::StorageError),
}
