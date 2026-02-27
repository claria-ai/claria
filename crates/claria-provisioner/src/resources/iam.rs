use std::future::Future;
use std::pin::Pin;

use aws_sdk_iam::Client;

use crate::error::ProvisionerError;
use crate::resource::{Resource, ResourceResult};

type Bf<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct IamRoleResource {
    client: Client,
    role_name: String,
    assume_role_policy: String,
}

impl IamRoleResource {
    pub fn new(client: Client, role_name: String, assume_role_policy: String) -> Self {
        Self {
            client,
            role_name,
            assume_role_policy,
        }
    }
}

impl Resource for IamRoleResource {
    fn resource_type(&self) -> &str {
        "iam_role"
    }

    fn current_state(&self) -> Bf<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self.client.get_role().role_name(&self.role_name).send().await {
                Ok(resp) => {
                    let role = resp.role();
                    Ok(Some(serde_json::json!({
                        "role_name": self.role_name,
                        "role_arn": role.map(|r| r.arn()),
                    })))
                }
                Err(_) => Ok(None),
            }
        })
    }

    fn create(&self) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        Box::pin(async {
            let result = self
                .client
                .create_role()
                .role_name(&self.role_name)
                .assume_role_policy_document(&self.assume_role_policy)
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(e.to_string()))?;

            let role_arn = result.role().map(|r| r.arn().to_string()).unwrap_or_default();
            tracing::info!(role_name = %self.role_name, role_arn = %role_arn, "IAM role created");

            Ok(ResourceResult {
                resource_id: role_arn.clone(),
                properties: serde_json::json!({
                    "role_name": self.role_name,
                    "role_arn": role_arn,
                }),
            })
        })
    }

    fn update(&self, _resource_id: &str) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        let rid = _resource_id.to_string();
        Box::pin(async move {
            self.client
                .update_assume_role_policy()
                .role_name(&self.role_name)
                .policy_document(&self.assume_role_policy)
                .send()
                .await
                .map_err(|e| ProvisionerError::UpdateFailed(e.to_string()))?;

            Ok(ResourceResult {
                resource_id: rid.clone(),
                properties: serde_json::json!({
                    "role_name": self.role_name,
                    "role_arn": rid,
                }),
            })
        })
    }

    fn delete(&self, _resource_id: &str) -> Bf<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            self.client
                .delete_role()
                .role_name(&self.role_name)
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(e.to_string()))?;
            tracing::info!(role_name = %self.role_name, "IAM role deleted");
            Ok(())
        })
    }
}
