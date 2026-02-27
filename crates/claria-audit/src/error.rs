use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("CloudTrail error: {0}")]
    CloudTrail(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("AWS config error: {0}")]
    Config(String),
}
