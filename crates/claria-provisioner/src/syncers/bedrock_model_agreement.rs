use aws_sdk_bedrock::types::AgreementStatus;
use aws_sdk_bedrock::Client;
use serde_json::json;

use crate::error::ProvisionerError;
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
}

impl ResourceSyncer for BedrockModelAgreementSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            let models = self
                .client
                .list_foundation_models()
                .send()
                .await
                .map_err(|e| ProvisionerError::Aws(e.to_string()))?;

            // Find a representative model matching this prefix
            let representative = models
                .model_summaries()
                .iter()
                .map(|m| m.model_id().to_string())
                .filter(|id| id.contains(self.model_prefix()))
                .find(|id| !is_context_window_variant(id));

            let Some(model_id) = representative else {
                return Ok(Some(json!({"agreement": "unavailable"})));
            };

            // Check agreement status
            let agreement = match self
                .client
                .get_foundation_model_availability()
                .model_id(&model_id)
                .send()
                .await
            {
                Ok(resp) => {
                    let needs_agreement = resp
                        .agreement_availability()
                        .map(|a| *a.status() == AgreementStatus::Available)
                        .unwrap_or(false);

                    if needs_agreement {
                        "pending"
                    } else {
                        "accepted"
                    }
                }
                Err(_) => "unknown",
            };

            Ok(Some(json!({
                "agreement": agreement,
                "model_id": model_id,
            })))
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
            let models = self
                .client
                .list_foundation_models()
                .send()
                .await
                .map_err(|e| ProvisionerError::Aws(e.to_string()))?;

            let matching_ids: Vec<String> = models
                .model_summaries()
                .iter()
                .map(|m| m.model_id().to_string())
                .filter(|id| {
                    id.contains(self.model_prefix()) && !is_context_window_variant(id)
                })
                .collect();

            for model_id in &matching_ids {
                // Check if agreement is pending
                let needs_agreement = match self
                    .client
                    .get_foundation_model_availability()
                    .model_id(model_id)
                    .send()
                    .await
                {
                    Ok(resp) => resp
                        .agreement_availability()
                        .map(|a| *a.status() == AgreementStatus::Available)
                        .unwrap_or(false),
                    Err(_) => continue,
                };

                if !needs_agreement {
                    continue;
                }

                // List offers and accept the first one
                let offers = match self
                    .client
                    .list_foundation_model_agreement_offers()
                    .model_id(model_id)
                    .send()
                    .await
                {
                    Ok(resp) => resp,
                    Err(e) => {
                        tracing::warn!(model_id, error = %e, "failed to list agreement offers");
                        continue;
                    }
                };

                if offers.offers().is_empty() {
                    continue;
                }

                let offer_token = offers.offers()[0].offer_token();
                tracing::info!(model_id, offer_token, "accepting model agreement");

                match self
                    .client
                    .create_foundation_model_agreement()
                    .model_id(model_id)
                    .offer_token(offer_token)
                    .send()
                    .await
                {
                    Ok(_) => tracing::info!(model_id, "model agreement accepted"),
                    Err(e) => {
                        tracing::warn!(model_id, error = %e, "failed to accept model agreement")
                    }
                }
            }

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
