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

            let available: Vec<String> = models
                .model_summaries()
                .iter()
                .filter_map(|m| {
                    let id = m.model_id().to_string();
                    if self.model_ids.iter().any(|wanted| id.contains(wanted)) {
                        Some(id)
                    } else {
                        None
                    }
                })
                .collect();

            if available.is_empty() {
                // Models not found — return Found with an error hint so the plan
                // can display a helpful message to the user.
                Ok(Some(serde_json::json!({
                    "available_models": [],
                    "error": format!(
                        "No Claude models found. Please enable model access in the AWS Bedrock console. \
                         Looking for models matching: {}",
                        self.model_ids.join(", ")
                    ),
                })))
            } else {
                Ok(Some(serde_json::json!({
                    "available_models": available,
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
