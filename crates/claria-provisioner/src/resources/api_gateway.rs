use std::future::Future;
use std::pin::Pin;

use aws_sdk_apigatewayv2::Client;

use crate::error::ProvisionerError;
use crate::resource::{Resource, ResourceResult};

type Bf<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct ApiGatewayResource {
    client: Client,
    api_name: String,
}

impl ApiGatewayResource {
    pub fn new(client: Client, api_name: String) -> Self {
        Self { client, api_name }
    }
}

impl Resource for ApiGatewayResource {
    fn resource_type(&self) -> &str {
        "api_gateway"
    }

    fn current_state(&self) -> Bf<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            let apis = self
                .client
                .get_apis()
                .send()
                .await
                .map_err(|e| ProvisionerError::Aws(e.to_string()))?;

            for api in apis.items() {
                if api.name() == Some(self.api_name.as_str()) {
                    return Ok(Some(serde_json::json!({
                        "api_name": self.api_name,
                        "api_id": api.api_id(),
                        "api_endpoint": api.api_endpoint().unwrap_or_default(),
                    })));
                }
            }
            Ok(None)
        })
    }

    fn create(&self) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        Box::pin(async {
            let result = self
                .client
                .create_api()
                .name(&self.api_name)
                .protocol_type(aws_sdk_apigatewayv2::types::ProtocolType::Http)
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(e.to_string()))?;

            let api_id = result.api_id().unwrap_or_default().to_string();
            let api_endpoint = result.api_endpoint().unwrap_or_default().to_string();

            tracing::info!(api_name = %self.api_name, api_id = %api_id, "API Gateway created");

            Ok(ResourceResult {
                resource_id: api_id.clone(),
                properties: serde_json::json!({
                    "api_name": self.api_name,
                    "api_id": api_id,
                    "api_endpoint": api_endpoint,
                }),
            })
        })
    }

    fn update(&self, resource_id: &str) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        let rid = resource_id.to_string();
        let name = self.api_name.clone();
        Box::pin(async move {
            Ok(ResourceResult {
                resource_id: rid.clone(),
                properties: serde_json::json!({ "api_name": name, "api_id": rid }),
            })
        })
    }

    fn delete(&self, resource_id: &str) -> Bf<'_, Result<(), ProvisionerError>> {
        let rid = resource_id.to_string();
        Box::pin(async move {
            self.client
                .delete_api()
                .api_id(&rid)
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(e.to_string()))?;
            tracing::info!(api_id = %rid, "API Gateway deleted");
            Ok(())
        })
    }
}
