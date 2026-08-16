use claria_core::s3_keys::log_safe_key;
use std::fmt;
use thiserror::Error;

/// An S3 object key that renders through [`log_safe_key`] in both `Display`
/// and `Debug`.
///
/// Every `StorageError` reaches the console ring buffer and the rolling
/// on-disk logs through the desktop command boundary's `error = ?error`, and a
/// `records/{uuid}/{filename}` key carries a client-chosen filename — PHI in a
/// HIPAA app. Redacting inside the key type means no log site has to remember
/// to scrub, and no future `%error` sink can reintroduce the leak. Code that
/// needs the real key reads [`LogSafeKey::as_str`].
///
/// Deliberately not `Serialize`: serializing would hand out the raw key and
/// undo the redaction the type exists to guarantee.
#[derive(Clone, PartialEq, Eq)]
pub struct LogSafeKey(String);

impl LogSafeKey {
    /// The unredacted key, for code that acts on the object.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LogSafeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&log_safe_key(&self.0))
    }
}

impl fmt::Debug for LogSafeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&log_safe_key(&self.0), f)
    }
}

impl From<&str> for LogSafeKey {
    fn from(key: &str) -> Self {
        Self(key.to_string())
    }
}

impl From<String> for LogSafeKey {
    fn from(key: String) -> Self {
        Self(key)
    }
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("object not found: {key}")]
    NotFound { key: LogSafeKey },

    #[error("precondition failed for key: {key}")]
    PreconditionFailed { key: LogSafeKey },

    #[error("conditional request conflict for key after retries: {key}")]
    ConditionalRequestConflict { key: LogSafeKey },

    /// A stored JSON object decoded but failed the caller's domain validation
    /// (see `state::load_state_checked`). `reason` is the caller's own
    /// user-facing sentence.
    #[error("invalid stored state at {key}: {reason}")]
    InvalidState { key: LogSafeKey, reason: String },

    #[error("object {key} is {actual_bytes} bytes, exceeding the {max_bytes}-byte read limit")]
    ObjectTooLarge {
        key: LogSafeKey,
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

    /// Restoring deleted objects under a prefix is per-key work; every key is
    /// attempted and the failures are aggregated so the caller knows exactly
    /// how much of the prefix is restored.
    #[error("restore left {failed} of {attempted} deleted objects unrestored: {details}")]
    RestoreObjects {
        attempted: usize,
        failed: usize,
        details: String,
    },

    #[error("S3 ListObjects error: {0}")]
    ListObjects(String),

    #[error("S3 ListObjectVersions error: {0}")]
    ListObjectVersions(String),
}
