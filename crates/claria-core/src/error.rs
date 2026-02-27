use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("invalid document type: {0}")]
    InvalidDocType(String),

    #[error("missing required field: {0}")]
    MissingField(String),

    #[error("invalid uuid: {0}")]
    InvalidUuid(#[from] uuid::Error),
}
