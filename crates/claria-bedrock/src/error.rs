use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BedrockError {
    #[error("model invocation failed: {0}")]
    Invocation(String),

    #[error("response parsing failed: {0}")]
    ResponseParse(String),

    #[error("response did not conform to expected schema: {0}")]
    SchemaViolation(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("model not supported: {0}")]
    UnsupportedModel(String),

    #[error("AWS config error: {0}")]
    Config(String),

    #[error("model agreement error: {0}")]
    Agreement(String),

    /// A model can't be invoked because access isn't set up for this account.
    /// Carries the bare foundation model id and a coarse reason the UI maps to
    /// an enrollment action (e.g. open the use-case form, or sign up for the
    /// model's agreement).
    #[error("model access required for {model_id}: {reason}")]
    ModelAccess {
        model_id: String,
        reason: ModelAccessReason,
    },
}

/// Why a model can't be invoked — coarse enough for the UI to route on without
/// re-parsing AWS error strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAccessReason {
    /// No marketplace agreement / subscription exists for this model yet.
    NotSubscribed,
    /// The Anthropic first-time-use form hasn't been submitted for the account.
    UseCaseFormRequired,
    /// The model must be invoked via an inference profile, not a bare model id.
    NeedsInferenceProfile,
    /// The account isn't authorized to invoke this model — AWS gates it
    /// ("not available for this account"). The model may be listed and even
    /// agreement-able, but enrollment can't unlock it; access must be requested
    /// from AWS (Bedrock console / AWS Sales).
    NotAuthorized,
    /// Access denied for a reason we couldn't classify.
    Unknown,
}

impl std::fmt::Display for ModelAccessReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ModelAccessReason::NotSubscribed => "not subscribed",
            ModelAccessReason::UseCaseFormRequired => "use-case form required",
            ModelAccessReason::NeedsInferenceProfile => "needs inference profile",
            ModelAccessReason::NotAuthorized => "not authorized",
            ModelAccessReason::Unknown => "unknown",
        };
        f.write_str(s)
    }
}
