use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("object not found: {key}")]
    NotFound { key: String },

    #[error("ETag mismatch (expected {expected}, got {actual})")]
    ETagMismatch { expected: String, actual: String },

    #[error("precondition failed for key: {key}")]
    PreconditionFailed { key: String },

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("S3 GetObject error: {0}")]
    GetObject(String),

    #[error("S3 PutObject error: {0}")]
    PutObject(String),

    #[error("S3 DeleteObject error: {0}")]
    DeleteObject(String),

    #[error("S3 ListObjects error: {0}")]
    ListObjects(String),

    #[error("S3 ListObjectVersions error: {0}")]
    ListObjectVersions(String),

    #[error("S3 presign error: {0}")]
    Presign(String),

    #[error("AWS config error: {0}")]
    Config(String),
}
