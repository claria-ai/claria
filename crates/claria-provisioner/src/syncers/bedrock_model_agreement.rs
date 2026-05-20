use aws_config::SdkConfig;
use aws_sdk_bedrock::Client as BedrockClient;
use aws_sdk_bedrockruntime::Client as RuntimeClient;
use claria_bedrock::availability::{self, InvocabilityProbe};
use serde_json::{json, Value};

use crate::error::{format_err_chain, ProvisionerError};
use crate::manifest::ResourceSpec;
use crate::syncer::{BoxFuture, ResourceSyncer};

/// Check whether a model ID is a context-window variant (e.g. `:48k`, `:200k`).
fn is_context_window_variant(model_id: &str) -> bool {
    model_id.rsplit_once(':').is_some_and(|(_, suffix)| {
        suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) && suffix != "0"
    })
}

pub struct BedrockModelAgreementSyncer {
    spec: ResourceSpec,
    bedrock: BedrockClient,
    runtime: RuntimeClient,
}

impl BedrockModelAgreementSyncer {
    pub fn new(spec: ResourceSpec, config: &SdkConfig) -> Self {
        Self {
            spec,
            bedrock: BedrockClient::new(config),
            runtime: RuntimeClient::new(config),
        }
    }

    fn model_prefix(&self) -> &str {
        &self.spec.resource_name
    }

    /// Enumerate foundation models matching this spec's prefix.
    async fn matching_model_ids(&self) -> Result<Vec<String>, ProvisionerError> {
        let models = self
            .bedrock
            .list_foundation_models()
            .send()
            .await
            .map_err(|e| {
                let msg = format_err_chain(&e);
                tracing::error!(error = %msg, "ListFoundationModels failed");
                ProvisionerError::Aws(msg)
            })?;

        Ok(models
            .model_summaries()
            .iter()
            .map(|m| m.model_id().to_string())
            .filter(|id| id.contains(self.model_prefix()) && !is_context_window_variant(id))
            .collect())
    }

    /// Classify each matching model as invokable or blocked using the shared
    /// CountTokens probe.
    ///
    /// The Bedrock catalog APIs (`GetFoundationModelAvailability`,
    /// `ListInferenceProfiles`, `GetInferenceProfile`) don't reliably
    /// distinguish invokable models from half-published preview entries —
    /// they happily report healthy state for models the runtime then
    /// rejects. The CountTokens probe is the only honest signal. See
    /// `claria_bedrock::availability` for the why.
    async fn classify_models(&self) -> Result<ModelAvailability, ProvisionerError> {
        let ids = self.matching_model_ids().await?;
        let probes = availability::probe_many(&self.runtime, &ids).await;

        let mut invokable: Vec<String> = Vec::new();
        let mut blocked: Vec<BlockedModel> = Vec::new();
        for (model_id, probe) in ids.into_iter().zip(probes) {
            match probe {
                InvocabilityProbe::Invokable => invokable.push(model_id),
                InvocabilityProbe::Rejected(reason) => {
                    blocked.push(BlockedModel { model_id, reason })
                }
            }
        }

        Ok(ModelAvailability { invokable, blocked })
    }

    /// Accept all available marketplace agreements for matching models.
    /// Idempotent — re-accepting an existing agreement is treated as success.
    async fn accept_agreements(&self) -> Result<(), ProvisionerError> {
        let ids = self.matching_model_ids().await?;
        let mut failures: Vec<String> = Vec::new();

        for model_id in &ids {
            let offers = match self
                .bedrock
                .list_foundation_model_agreement_offers()
                .model_id(model_id)
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    let msg = format_err_chain(&e);
                    tracing::warn!(model_id, error = %msg, "failed to list agreement offers");
                    failures.push(format!("{model_id}: {msg}"));
                    continue;
                }
            };

            // Empty offers can mean (a) the agreement was already accepted on
            // a prior run, or (b) no marketplace agreement applies to this
            // model. AWS doesn't expose a "list accepted agreements" API, so
            // we can't tell which. Either way there's nothing for us to do
            // here — the CountTokens probe in classify_models is the
            // authoritative signal for invokability.
            if offers.offers().is_empty() {
                continue;
            }

            let offer_token = offers.offers()[0].offer_token();

            match self
                .bedrock
                .create_foundation_model_agreement()
                .model_id(model_id)
                .offer_token(offer_token)
                .send()
                .await
            {
                Ok(_) => {
                    tracing::info!(model_id, "model agreement accepted");
                }
                Err(e) => {
                    let msg = format_err_chain(&e);
                    if msg.contains("already exists") {
                        tracing::debug!(model_id, "model agreement already accepted");
                    } else {
                        tracing::warn!(model_id, error = %msg, "failed to accept model agreement");
                        failures.push(format!("{model_id}: {msg}"));
                    }
                }
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(ProvisionerError::Aws(failures.join("; ")))
        }
    }
}

/// One unavailable model and the reason it can't be invoked.
struct BlockedModel {
    model_id: String,
    reason: String,
}

/// Result of classifying matching models against the account's runtime state.
struct ModelAvailability {
    invokable: Vec<String>,
    blocked: Vec<BlockedModel>,
}

impl ModelAvailability {
    fn into_json(self) -> Value {
        let agreement = if self.blocked.is_empty() {
            "accepted"
        } else {
            "blocked"
        };
        json!({
            "agreement": agreement,
            "invokable_models": self.invokable,
            "blocked_models": self
                .blocked
                .into_iter()
                .map(|b| json!({"model_id": b.model_id, "reason": b.reason}))
                .collect::<Vec<_>>(),
        })
    }
}

impl ResourceSyncer for BedrockModelAgreementSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            let availability = self.classify_models().await?;
            Ok(Some(availability.into_json()))
        })
    }

    /// Reduce `read()`'s rich state to a single comparable field so drift
    /// detection trips when any model is blocked.
    fn current_state(&self, actual: &Value) -> Value {
        json!({
            "agreement": actual
                .get("agreement")
                .cloned()
                .unwrap_or(Value::Null),
        })
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            // Try to accept any pending marketplace agreements — this is the
            // only mutation we can do without user intervention.
            self.accept_agreements().await?;

            // Re-probe. If any model is still blocked, surface the reasons
            // so the UI explains what the user must do manually.
            let availability = self.classify_models().await?;
            if !availability.blocked.is_empty() {
                let summary = availability
                    .blocked
                    .iter()
                    .map(|b| format!("{}: {}", b.model_id, b.reason))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(ProvisionerError::Aws(format!(
                    "Bedrock models still blocked after accepting agreements — {summary}"
                )));
            }
            Ok(availability.into_json())
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        self.create()
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        // Can't un-accept a model agreement
        Box::pin(async { Ok(()) })
    }
}
