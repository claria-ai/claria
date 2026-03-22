use aws_sdk_iam::Client;
use serde_json::json;

use crate::account_setup::{self, IAM_USER_NAME};
use crate::error::ProvisionerError;
use crate::manifest::ResourceSpec;
use crate::syncer::{BoxFuture, ResourceSyncer};

pub struct IamUserSyncer {
    spec: ResourceSpec,
    client: Client,
}

impl IamUserSyncer {
    pub fn new(spec: ResourceSpec, client: Client) -> Self {
        Self { spec, client }
    }
}

impl ResourceSyncer for IamUserSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self
                .client
                .get_user()
                .user_name(IAM_USER_NAME)
                .send()
                .await
            {
                Ok(resp) => {
                    let arn = resp
                        .user()
                        .map(|u| u.arn().to_string())
                        .unwrap_or_default();
                    Ok(Some(json!({"exists": true, "user_arn": arn})))
                }
                Err(e) => {
                    let is_not_found = e
                        .as_service_error()
                        .map(|se| se.is_no_such_entity_exception())
                        .unwrap_or(false);
                    if is_not_found {
                        return Ok(None);
                    }
                    // Access denied during scan is fine — we just can't read
                    // this resource with the current credentials.
                    let is_access_denied = e
                        .as_service_error()
                        .map(|se| se.meta().code() == Some("AccessDenied"))
                        .unwrap_or(false);
                    if is_access_denied {
                        return Ok(None);
                    }
                    Err(ProvisionerError::Aws(format!(
                        "iam:GetUser failed: {e}"
                    )))
                }
            }
        })
    }

    fn current_state(&self, _actual: &serde_json::Value) -> serde_json::Value {
        // read() returns {exists, user_arn} — only compare the exists flag
        self.spec().desired.clone()
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            let user_arn = account_setup::create_user(&self.client).await?;
            Ok(json!({"exists": true, "user_arn": user_arn}))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            // User itself doesn't change — drift only triggers on existence.
            Ok(json!({"exists": true}))
        })
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            // Delete all access keys first, then the user.
            let keys = self
                .client
                .list_access_keys()
                .user_name(IAM_USER_NAME)
                .send()
                .await
                .map_err(|e| {
                    ProvisionerError::Aws(format!("iam:ListAccessKeys failed: {e}"))
                })?;

            for meta in keys.access_key_metadata() {
                if let Some(key_id) = meta.access_key_id() {
                    self.client
                        .delete_access_key()
                        .user_name(IAM_USER_NAME)
                        .access_key_id(key_id)
                        .send()
                        .await
                        .map_err(|e| {
                            ProvisionerError::Aws(format!(
                                "iam:DeleteAccessKey failed: {e}"
                            ))
                        })?;
                }
            }

            self.client
                .delete_user()
                .user_name(IAM_USER_NAME)
                .send()
                .await
                .map_err(|e| {
                    ProvisionerError::Aws(format!("iam:DeleteUser failed: {e}"))
                })?;

            tracing::info!("deleted IAM user {IAM_USER_NAME}");
            Ok(())
        })
    }
}
