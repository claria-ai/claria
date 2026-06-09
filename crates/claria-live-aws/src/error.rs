use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("AWS API error during reconcile: {0}")]
    AwsApi(String),

    #[error("IAM permission missing: {0}")]
    PermissionDenied(String),

    #[error("provider key {key:?} not registered")]
    UnknownKey { key: String },

    #[error("provider {key:?} does not accept this apply: {reason}")]
    InvalidApply { key: String, reason: String },

    #[error("provisioner error: {0}")]
    Provisioner(#[from] claria_provisioner::ProvisionerError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
