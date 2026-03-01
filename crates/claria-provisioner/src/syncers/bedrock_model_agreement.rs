use aws_sdk_bedrock::Client;
use serde_json::json;

use crate::error::{format_err_chain, ProvisionerError};
use crate::manifest::{FieldDrift, ResourceSpec};
use crate::syncer::{BoxFuture, ResourceSyncer};

/// Check whether a model ID is a context-window variant (e.g. `:48k`, `:200k`).
fn is_context_window_variant(model_id: &str) -> bool {
    model_id.rsplit_once(':').is_some_and(|(_, suffix)| {
        suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) && suffix != "0"
    })
}

pub struct BedrockModelAgreementSyncer {
    spec: ResourceSpec,
    client: Client,
}

impl BedrockModelAgreementSyncer {
    pub fn new(spec: ResourceSpec, client: Client) -> Self {
        Self { spec, client }
    }

    fn model_prefix(&self) -> &str {
        &self.spec.resource_name
    }

    /// Ensure all matching models have their Marketplace agreements accepted.
    ///
    /// This is idempotent: if an agreement already exists, AWS returns a
    /// `ValidationException` with "Agreement already exists" which we treat
    /// as success.
    ///
    /// We skip `GetFoundationModelAvailability` because its `Available` status
    /// means "an agreement mechanism exists" — NOT "the agreement hasn't been
    /// accepted yet". Directly attempting acceptance is the only reliable check.
    async fn ensure_agreements(&self) -> Result<(), ProvisionerError> {
        let models = self
            .client
            .list_foundation_models()
            .send()
            .await
            .map_err(|e| ProvisionerError::Aws(format_err_chain(&e)))?;

        let matching_ids: Vec<String> = models
            .model_summaries()
            .iter()
            .map(|m| m.model_id().to_string())
            .filter(|id| id.contains(self.model_prefix()) && !is_context_window_variant(id))
            .collect();

        let mut failures: Vec<String> = Vec::new();

        for model_id in &matching_ids {
            let offers = match self
                .client
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

            if offers.offers().is_empty() {
                continue;
            }

            let offer_token = offers.offers()[0].offer_token();

            match self
                .client
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

impl ResourceSyncer for BedrockModelAgreementSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self.ensure_agreements().await {
                Ok(()) => Ok(Some(json!({"agreement": "accepted"}))),
                Err(_) => Ok(Some(json!({"agreement": "pending"}))),
            }
        })
    }

    fn diff(&self, actual: &serde_json::Value) -> Vec<FieldDrift> {
        let status = actual
            .get("agreement")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        if status == "accepted" {
            vec![]
        } else {
            vec![FieldDrift {
                field: "agreement".into(),
                label: "Model agreement".into(),
                expected: json!("accepted"),
                actual: json!(status),
            }]
        }
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            self.ensure_agreements().await?;
            Ok(json!({"agreement": "accepted"}))
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
