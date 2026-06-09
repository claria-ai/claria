use std::collections::HashMap;

use aws_config::SdkConfig;
use serde::{Deserialize, Serialize};
use specta::Type;

use claria_provisioner::{ManifestContributor, ResourceSpec};

use crate::error::ProviderError;
use crate::provider::{Candidate, ChangeReason, LiveAwsValueProvider, ProviderKind, Reconciliation};

/// Collection of registered providers. Built once at app startup and lives
/// on `DesktopState`.
pub struct LiveAwsValueRegistry {
    providers: Vec<Box<dyn LiveAwsValueProvider>>,
}

impl LiveAwsValueRegistry {
    pub fn new() -> Self {
        Self { providers: vec![] }
    }

    pub fn with_provider(mut self, provider: Box<dyn LiveAwsValueProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    pub fn provider(&self, key: &str) -> Option<&dyn LiveAwsValueProvider> {
        self.providers.iter().find(|p| p.key() == key).map(|p| &**p)
    }

    /// All registered keys.
    pub fn keys(&self) -> Vec<&'static str> {
        self.providers.iter().map(|p| p.key()).collect()
    }

    /// Read-only reconciliation across every registered provider. Uses
    /// `current_values` to feed each provider's stored state; `Value::Null`
    /// for any provider that has no stored value yet.
    ///
    /// Returns the report plus a list of `ManifestContributor` adapters for
    /// every `ProvisionerContributor` provider that reported `Contribution`,
    /// so the caller can feed them into `build_manifest_with_contributors`.
    pub async fn reconcile_all(
        &self,
        sdk: &SdkConfig,
        current_values: &HashMap<String, serde_json::Value>,
    ) -> (ReconciliationReport, Vec<Box<dyn ManifestContributor>>) {
        let mut report = ReconciliationReport {
            in_sync: vec![],
            user_choice_drift: vec![],
            contribution_summaries: vec![],
            errors: vec![],
            checked_at: jiff::Timestamp::now().to_string(),
        };
        let mut contributors: Vec<Box<dyn ManifestContributor>> = vec![];

        for provider in &self.providers {
            let null = serde_json::Value::Null;
            let current = current_values.get(provider.key()).unwrap_or(&null);

            match provider.reconcile(sdk, current).await {
                Ok(Reconciliation::InSync) => {
                    report.in_sync.push(provider.key().to_string());
                }
                Ok(Reconciliation::UserChoiceDrift {
                    reason,
                    candidates,
                    current: drift_current,
                    summary,
                }) => {
                    report.user_choice_drift.push(UserChoiceDriftEntry {
                        key: provider.key().to_string(),
                        label: provider.label().to_string(),
                        reason,
                        candidates,
                        current: drift_current,
                        summary,
                    });
                }
                Ok(Reconciliation::Contribution {
                    reason,
                    specs,
                    summary,
                }) => {
                    report.contribution_summaries.push(ContributionSummary {
                        key: provider.key().to_string(),
                        reason,
                        summary,
                        spec_count: specs.len(),
                    });
                    contributors.push(Box::new(FixedSpecContributor { specs }));
                }
                Err(e) => {
                    report.errors.push(ProviderErrorEntry {
                        key: provider.key().to_string(),
                        message: e.to_string(),
                    });
                }
            }
        }

        (report, contributors)
    }

    /// Apply a `UserChoice` plan — writes the user's pick into the supplied
    /// `expiring_values` map and returns the updated map. The caller is
    /// responsible for persisting it via `save_config`.
    pub fn apply_user_choice(
        &self,
        key: &str,
        choice: serde_json::Value,
        expiring_values: &mut HashMap<String, serde_json::Value>,
    ) -> Result<(), ProviderError> {
        let provider = self.provider(key).ok_or(ProviderError::UnknownKey {
            key: key.to_string(),
        })?;
        if provider.kind() != ProviderKind::UserChoice {
            return Err(ProviderError::InvalidApply {
                key: key.to_string(),
                reason: "this provider's drift is resolved via the provisioner pipeline".into(),
            });
        }
        expiring_values.insert(key.to_string(), choice);
        Ok(())
    }
}

impl Default for LiveAwsValueRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Adapter that lets us hand a precomputed `Vec<ResourceSpec>` to
/// `build_manifest_with_contributors` without re-running the provider.
/// Avoids re-querying AWS between reconcile and apply.
struct FixedSpecContributor {
    specs: Vec<ResourceSpec>,
}

impl ManifestContributor for FixedSpecContributor {
    fn contribute<'a>(
        &'a self,
        _sdk: &'a SdkConfig,
    ) -> claria_provisioner::syncer::BoxFuture<
        'a,
        Result<Vec<ResourceSpec>, claria_provisioner::ProvisionerError>,
    > {
        let specs = self.specs.clone();
        Box::pin(async move { Ok(specs) })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ReconciliationReport {
    pub in_sync: Vec<String>,
    pub user_choice_drift: Vec<UserChoiceDriftEntry>,
    pub contribution_summaries: Vec<ContributionSummary>,
    pub errors: Vec<ProviderErrorEntry>,
    /// RFC3339 timestamp when the reconciliation completed.
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UserChoiceDriftEntry {
    pub key: String,
    pub label: String,
    pub reason: ChangeReason,
    pub candidates: Vec<Candidate>,
    pub current: Option<serde_json::Value>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ContributionSummary {
    pub key: String,
    pub reason: ChangeReason,
    pub summary: String,
    pub spec_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProviderErrorEntry {
    pub key: String,
    pub message: String,
}
