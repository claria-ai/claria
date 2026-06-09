use std::future::Future;
use std::pin::Pin;

use aws_config::SdkConfig;
use serde::{Deserialize, Serialize};
use specta::Type;

use claria_provisioner::ResourceSpec;

use crate::error::ProviderError;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A value in Claria's configuration whose source of truth is AWS, not the
/// Claria source code. Providers expose:
/// - read-only `reconcile` against AWS to detect drift
/// - depending on kind, either a `UserChoice` drift the user resolves by
///   picking a candidate, or a `Contribution` of `ResourceSpec`s that the
///   provisioner pipeline turns into create/modify actions
///
/// Implementations MUST be read-only inside `reconcile`. Any AWS-side writes
/// belong in syncers run by the provisioner under elevated credentials.
pub trait LiveAwsValueProvider: Send + Sync {
    /// Stable key, e.g. `"bedrock.chat_model"`. Used as the config map key
    /// and in `Manifest::contributors`. Never changes.
    fn key(&self) -> &'static str;

    /// Human-friendly title.
    fn label(&self) -> &'static str;

    /// One-paragraph explanation of what this value does.
    fn description(&self) -> &'static str;

    /// Which apply path this provider uses.
    fn kind(&self) -> ProviderKind;

    /// Read-only reconciliation against AWS reality. Returns drift if any.
    ///
    /// `current` is the value currently stored in `ClariaConfig.expiring_values`
    /// (or `Value::Null` if absent). For `ProvisionerContributor` kinds it is
    /// always `Value::Null` — those providers have no per-user stored state.
    fn reconcile<'a>(
        &'a self,
        sdk: &'a SdkConfig,
        current: &'a serde_json::Value,
    ) -> BoxFuture<'a, Result<Reconciliation, ProviderError>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Drift resolved by writing to local config under scoped creds.
    UserChoice,
    /// Drift resolved by contributing `ResourceSpec`s to the provisioner
    /// manifest; apply runs under elevation through the existing
    /// provisioner pipeline.
    ProvisionerContributor,
}

/// Outcome of a single provider's `reconcile`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Reconciliation {
    /// Current state matches AWS reality — nothing to do.
    InSync,

    /// User picks one of the candidates; framework writes the result to
    /// `expiring_values[key]` under scoped creds.
    UserChoiceDrift {
        reason: ChangeReason,
        candidates: Vec<Candidate>,
        current: Option<serde_json::Value>,
        summary: String,
    },

    /// These specs should be folded into the provisioner manifest. The
    /// provisioner's normal plan/apply pipeline turns them into the
    /// concrete diff the user sees and authorizes.
    Contribution {
        reason: ChangeReason,
        specs: Vec<ResourceSpec>,
        summary: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ChangeReason {
    /// Stored value points at something AWS removed.
    Retired,
    /// Stored value still works but is in legacy/EOL window.
    /// `eol` is an RFC3339 timestamp (Tauri wire convention).
    Deprecation { eol: String },
    /// Stored value doesn't match the user's geography (after a Preferences change).
    PreferenceMismatch,
    /// First run — no value stored yet.
    NeverConfigured,
    /// Stored value is malformed or unrecognizable.
    Malformed,
    /// Forward-looking: AWS has new resources Claria could use.
    NewUpstreamResource { name: String },
}

/// A candidate value the user can pick from for `UserChoiceDrift`.
///
/// `metadata` is free-form per-provider — the chat-model provider uses it
/// to expose scope/region/price-tier; the transcribe-language provider uses
/// it for the human-readable language name.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Candidate {
    pub value: serde_json::Value,
    pub label: String,
    pub description: String,
    pub recommended: bool,
    pub metadata: serde_json::Value,
}
