use std::future::Future;
use std::pin::Pin;

use aws_sdk_bedrock::types::AgreementStatus;
use aws_sdk_bedrock::Client;

use crate::error::ProvisionerError;
use crate::resource::{Resource, ResourceResult};

type Bf<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct BedrockAccessResource {
    client: Client,
    model_ids: Vec<String>,
}

impl BedrockAccessResource {
    pub fn new(client: Client, model_ids: Vec<String>) -> Self {
        Self { client, model_ids }
    }
}

impl Resource for BedrockAccessResource {
    fn resource_type(&self) -> &str {
        "bedrock_model_access"
    }

    fn current_state(&self) -> Bf<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            let models = self
                .client
                .list_foundation_models()
                .send()
                .await
                .map_err(|e| ProvisionerError::Aws(e.to_string()))?;

            let all_model_ids: Vec<String> = models
                .model_summaries()
                .iter()
                .map(|m| m.model_id().to_string())
                .collect();

            // For each wanted prefix, find matching models and check agreement status
            let mut families: Vec<serde_json::Value> = Vec::new();
            let mut all_available: Vec<String> = Vec::new();
            let mut missing: Vec<String> = Vec::new();

            for wanted in &self.model_ids {
                let matches: Vec<String> = all_model_ids
                    .iter()
                    .filter(|id| id.contains(wanted.as_str()))
                    .cloned()
                    .collect();

                let found = !matches.is_empty();

                // Check agreement status using a representative model from this family.
                // Pick the first base model ID (no context-window suffix like :48k).
                let agreement = if found {
                    let representative = matches
                        .iter()
                        .find(|id| !is_context_window_variant(id))
                        .or(matches.first());

                    if let Some(model_id) = representative {
                        check_agreement_status(&self.client, model_id).await
                    } else {
                        "unknown".to_string()
                    }
                } else {
                    "unknown".to_string()
                };

                families.push(serde_json::json!({
                    "prefix": wanted,
                    "available": found,
                    "models": matches,
                    "agreement": agreement,
                }));

                if found {
                    all_available.extend(matches);
                } else {
                    missing.push(wanted.clone());
                }
            }

            if !missing.is_empty() {
                Ok(Some(serde_json::json!({
                    "available_models": all_available,
                    "families": families,
                    "error": format!(
                        "Missing model access. Please enable these in the AWS Bedrock console: {}",
                        missing.join(", ")
                    ),
                })))
            } else {
                Ok(Some(serde_json::json!({
                    "available_models": all_available,
                    "families": families,
                })))
            }
        })
    }

    fn create(&self) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        Box::pin(async move {
            accept_pending_agreements(&self.client, &self.model_ids).await?;

            tracing::info!(models = ?self.model_ids, "Bedrock model access verified");
            Ok(ResourceResult {
                resource_id: "bedrock_access".to_string(),
                properties: serde_json::json!({ "model_ids": self.model_ids }),
            })
        })
    }

    fn update(&self, resource_id: &str) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        let rid = resource_id.to_string();
        Box::pin(async move {
            accept_pending_agreements(&self.client, &self.model_ids).await?;

            tracing::info!(models = ?self.model_ids, "Bedrock model access re-verified");
            Ok(ResourceResult {
                resource_id: rid,
                properties: serde_json::json!({ "model_ids": self.model_ids }),
            })
        })
    }

    fn delete(&self, _resource_id: &str) -> Bf<'_, Result<(), ProvisionerError>> {
        Box::pin(async { Ok(()) })
    }
}

/// Check whether a model ID is a context-window variant (e.g. `:48k`, `:200k`).
fn is_context_window_variant(model_id: &str) -> bool {
    model_id.rsplit_once(':').is_some_and(|(_, suffix)| {
        suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) && suffix != "0"
    })
}

/// Check the Marketplace agreement status for a foundation model.
///
/// Returns `"accepted"`, `"pending"`, or `"unknown"`.
async fn check_agreement_status(client: &Client, model_id: &str) -> String {
    match client
        .get_foundation_model_availability()
        .model_id(model_id)
        .send()
        .await
    {
        Ok(resp) => {
            let needs_agreement = resp
                .agreement_availability()
                .map(|a| *a.status() == AgreementStatus::Available)
                .unwrap_or(false);

            if needs_agreement {
                "pending".to_string()
            } else {
                "accepted".to_string()
            }
        }
        Err(e) => {
            tracing::warn!(model_id, error = %e, "failed to check agreement status");
            "unknown".to_string()
        }
    }
}

/// Accept Marketplace agreements for all models matching the given prefixes.
///
/// Lists all foundation models, finds those matching the prefixes, checks
/// which have pending agreements, and accepts them.
async fn accept_pending_agreements(
    client: &Client,
    model_prefixes: &[String],
) -> Result<(), ProvisionerError> {
    let models = client
        .list_foundation_models()
        .send()
        .await
        .map_err(|e| ProvisionerError::Aws(e.to_string()))?;

    // Collect base model IDs (skip context-window variants) matching our prefixes.
    let matching_ids: Vec<String> = models
        .model_summaries()
        .iter()
        .map(|m| m.model_id().to_string())
        .filter(|id| {
            model_prefixes.iter().any(|prefix| id.contains(prefix.as_str()))
                && !is_context_window_variant(id)
        })
        .collect();

    for model_id in &matching_ids {
        // Check if agreement is pending.
        let needs_agreement = match client
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

        // List offers and accept the first one.
        let offers = match client
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

        match client
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
                // Non-fatal: log and continue with other models.
                tracing::warn!(model_id, error = %e, "failed to accept model agreement");
            }
        }
    }

    Ok(())
}
