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
