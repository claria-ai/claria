use aws_sdk_iam::Client;
use serde_json::json;

use crate::account_setup::IAM_USER_NAME;
use crate::error::ProvisionerError;
use crate::manifest::{FieldDrift, ResourceSpec};
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
                    Err(ProvisionerError::Aws(format!(
                        "iam:GetUser failed: {e}"
                    )))
                }
            }
        })
    }

    fn diff(&self, _actual: &serde_json::Value) -> Vec<FieldDrift> {
        // Binary: user exists or not. If read() returned Some, it exists.
        vec![]
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "IAM user is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "IAM user is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "IAM user is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }
}
