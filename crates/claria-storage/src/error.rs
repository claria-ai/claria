use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("object not found: {key}")]
    NotFound { key: String },

    #[error("ETag mismatch (expected {expected}, got {actual})")]
    ETagMismatch { expected: String, actual: String },

    #[error("precondition failed for key: {key}")]
    PreconditionFailed { key: String },

    #[error("conditional request conflict for key after retries: {key}")]
    ConditionalRequestConflict { key: String },

    /// A stored JSON object decoded but failed the caller's domain validation
    /// (see `state::load_state_checked`). `reason` is the caller's own
    /// user-facing sentence.
    #[error("invalid stored state at {key}: {reason}")]
    InvalidState { key: String, reason: String },

    #[error("object {key} is {actual_bytes} bytes, exceeding the {max_bytes}-byte read limit")]
    ObjectTooLarge {
        key: String,
        actual_bytes: u64,
        max_bytes: u64,
    },

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("S3 GetObject error: {0}")]
    GetObject(String),

    #[error("S3 PutObject error: {0}")]
    PutObject(String),

    #[error("S3 DeleteObject error: {0}")]
    DeleteObject(String),

    /// S3 reports per-object failures inside a 200 response to DeleteObjects,
    /// so a batch that reports success at the HTTP level can still have left
    /// objects behind.
    #[error("S3 DeleteObjects left {failed} of {attempted} objects in place: {details}")]
    DeleteObjects {
        attempted: usize,
        failed: usize,
        details: String,
    },

    #[error("S3 ListObjects error: {0}")]
    ListObjects(String),

    #[error("S3 ListObjectVersions error: {0}")]
    ListObjectVersions(String),

    #[error("S3 presign error: {0}")]
    Presign(String),

    #[error("AWS config error: {0}")]
    Config(String),
}
