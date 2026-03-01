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

/// Walk the full error chain and join all causes into one string.
///
/// AWS SDK errors often have terse `Display` impls (e.g. "service error")
/// but useful detail in the source chain.
pub fn format_err_chain(err: &dyn std::error::Error) -> String {
    let mut msg = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        msg.push_str(": ");
        msg.push_str(&cause.to_string());
        source = cause.source();
    }
    msg
}
