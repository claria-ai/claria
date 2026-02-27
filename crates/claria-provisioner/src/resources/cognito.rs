use std::future::Future;
use std::pin::Pin;

use aws_sdk_cognitoidentityprovider::Client;

use crate::error::ProvisionerError;
use crate::resource::{Resource, ResourceResult};

type Bf<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct CognitoResource {
    client: Client,
    pool_name: String,
}

impl CognitoResource {
    pub fn new(client: Client, pool_name: String) -> Self {
        Self { client, pool_name }
    }
}

impl Resource for CognitoResource {
    fn resource_type(&self) -> &str {
        "cognito_user_pool"
    }

    fn current_state(&self) -> Bf<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            let pools = self
                .client
                .list_user_pools()
                .max_results(60)
                .send()
                .await
                .map_err(|e| ProvisionerError::Aws(e.to_string()))?;

            for pool in pools.user_pools() {
                if pool.name() == Some(self.pool_name.as_str()) {
                    return Ok(Some(serde_json::json!({
                        "pool_name": self.pool_name,
                        "pool_id": pool.id().unwrap_or_default(),
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
                .create_user_pool()
                .pool_name(&self.pool_name)
                .mfa_configuration(
                    aws_sdk_cognitoidentityprovider::types::UserPoolMfaType::Optional,
                )
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(e.to_string()))?;

            let pool_id = result
                .user_pool()
                .and_then(|p| p.id())
                .unwrap_or_default()
                .to_string();

            tracing::info!(pool_name = %self.pool_name, pool_id = %pool_id, "Cognito user pool created");

            Ok(ResourceResult {
                resource_id: pool_id.clone(),
                properties: serde_json::json!({
                    "pool_name": self.pool_name,
                    "pool_id": pool_id,
                }),
            })
        })
    }

    fn update(&self, resource_id: &str) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        let rid = resource_id.to_string();
        let pool_name = self.pool_name.clone();
        Box::pin(async move {
            Ok(ResourceResult {
                resource_id: rid.clone(),
                properties: serde_json::json!({ "pool_name": pool_name, "pool_id": rid }),
            })
        })
    }

    fn delete(&self, resource_id: &str) -> Bf<'_, Result<(), ProvisionerError>> {
        let rid = resource_id.to_string();
        Box::pin(async move {
            self.client
                .delete_user_pool()
                .user_pool_id(&rid)
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(e.to_string()))?;
            tracing::info!(pool_id = %rid, "Cognito user pool deleted");
            Ok(())
        })
    }
}
