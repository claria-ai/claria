use std::future::Future;
use std::pin::Pin;

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

            // For each wanted prefix, find matching models and report per-family
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
                families.push(serde_json::json!({
                    "prefix": wanted,
                    "available": found,
                    "models": matches,
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
        let model_ids = self.model_ids.clone();
        Box::pin(async move {
            // Bedrock model access can't be enabled programmatically.
            // This "create" just records the verification state.
            tracing::info!(models = ?model_ids, "Bedrock model access verified");
            Ok(ResourceResult {
                resource_id: "bedrock_access".to_string(),
                properties: serde_json::json!({ "model_ids": model_ids }),
            })
        })
    }

    fn update(&self, resource_id: &str) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        let rid = resource_id.to_string();
        let model_ids = self.model_ids.clone();
        Box::pin(async move {
            tracing::info!(models = ?model_ids, "Bedrock model access re-verified");
            Ok(ResourceResult {
                resource_id: rid,
                properties: serde_json::json!({ "model_ids": model_ids }),
            })
        })
    }

    fn delete(&self, _resource_id: &str) -> Bf<'_, Result<(), ProvisionerError>> {
        Box::pin(async { Ok(()) })
    }
}
