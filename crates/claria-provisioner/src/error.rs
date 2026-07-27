use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProvisionerError {
    #[error("resource not found: {resource_type}/{resource_id}")]
    ResourceNotFound {
        resource_type: String,
        resource_id: String,
    },

    #[error("resource creation failed: {0}")]
    CreateFailed(String),

    #[error("resource update failed: {0}")]
    UpdateFailed(String),

    #[error("resource deletion failed: {0}")]
    DeleteFailed(String),

    #[error("drift detected: {0}")]
    DriftDetected(String),

    #[error("state error: {0}")]
    State(String),

    #[error("AWS error: {0}")]
    Aws(String),

    /// IAM caps every user at two access keys. Kept separate from
    /// [`Self::Aws`] so the caller can offer the operator a way to free a
    /// slot instead of dead-ending on an opaque service error.
    #[error(
        "the {user_name} IAM user already has the maximum of {limit} access keys — \
         delete one to make room for this computer"
    )]
    AccessKeyLimitExceeded { user_name: String, limit: u32 },

    #[error("timed out after {seconds}s waiting for {operation}: {last_error}")]
    Timeout {
        operation: String,
        seconds: u64,
        last_error: String,
    },

    #[error("storage error: {0}")]
    Storage(#[from] claria_storage::error::StorageError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl ProvisionerError {
    /// Prepend resource identity to the error message.
    pub fn with_resource(self, label: &str, name: &str) -> Self {
        match self {
            Self::CreateFailed(msg) => Self::CreateFailed(format!("{label} ({name}): {msg}")),
            Self::UpdateFailed(msg) => Self::UpdateFailed(format!("{label} ({name}): {msg}")),
            Self::DeleteFailed(msg) => Self::DeleteFailed(format!("{label} ({name}): {msg}")),
            Self::Aws(msg) => Self::Aws(format!("{label} ({name}): {msg}")),
            other => other,
        }
    }
}
