use std::future::Future;
use std::pin::Pin;

use aws_sdk_iam::Client;

use crate::account_setup::{IAM_POLICY_NAME, IAM_USER_NAME};
use crate::error::ProvisionerError;
use crate::resource::{Resource, ResourceResult};

type Bf<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Read-only verification resource for the `claria-admin` IAM user and its
/// attached policy.
///
/// The IAM user is created during bootstrap (in `account_setup`), not during
/// the provisioning pipeline. This resource exists so the dashboard can show
/// whether the user and policy are intact and let the operator inspect the
/// policy document.
pub struct IamUserResource {
    client: Client,
}

impl IamUserResource {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Fetch the policy document JSON for a given policy ARN.
    async fn fetch_policy_document(
        &self,
        policy_arn: &str,
    ) -> Result<Option<serde_json::Value>, ProvisionerError> {
        // Get the policy to find the default version ID.
        let policy_resp = self
            .client
            .get_policy()
            .policy_arn(policy_arn)
            .send()
            .await
            .map_err(|e| ProvisionerError::Aws(format!("iam:GetPolicy failed: {e}")))?;

        let version_id = policy_resp
            .policy()
            .and_then(|p| p.default_version_id())
            .unwrap_or("v1");

        // Fetch the actual document for that version.
        let version_resp = self
            .client
            .get_policy_version()
            .policy_arn(policy_arn)
            .version_id(version_id)
            .send()
            .await
            .map_err(|e| ProvisionerError::Aws(format!("iam:GetPolicyVersion failed: {e}")))?;

        let doc_str = version_resp
            .policy_version()
            .and_then(|v| v.document())
            .unwrap_or("");

        // IAM returns the document URL-encoded.
        let decoded = percent_encoding::percent_decode_str(doc_str)
            .decode_utf8()
            .unwrap_or_default();
        let parsed: serde_json::Value =
            serde_json::from_str(&decoded).unwrap_or(serde_json::Value::Null);

        if parsed.is_null() {
            Ok(None)
        } else {
            Ok(Some(parsed))
        }
    }
}

impl Resource for IamUserResource {
    fn resource_type(&self) -> &str {
        "iam_user"
    }

    fn expected_id(&self) -> Option<&str> {
        Some(IAM_USER_NAME)
    }

    fn current_state(&self) -> Bf<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            // Step 1: Does the user exist?
            let user_resp = match self
                .client
                .get_user()
                .user_name(IAM_USER_NAME)
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    let is_not_found = e
                        .as_service_error()
                        .map(|se| se.is_no_such_entity_exception())
                        .unwrap_or(false);
                    if is_not_found {
                        return Ok(None);
                    }
                    return Err(ProvisionerError::Aws(format!(
                        "iam:GetUser failed: {e}"
                    )));
                }
            };

            let user_arn = user_resp
                .user()
                .map(|u| u.arn().to_string())
                .unwrap_or_default();

            // Step 2: Check attached policies for ClariaProvisionerAccess.
            let policies_resp = self
                .client
                .list_attached_user_policies()
                .user_name(IAM_USER_NAME)
                .send()
                .await
                .map_err(|e| {
                    ProvisionerError::Aws(format!(
                        "iam:ListAttachedUserPolicies failed: {e}"
                    ))
                })?;

            let claria_policy = policies_resp
                .attached_policies()
                .iter()
                .find(|p| p.policy_name() == Some(IAM_POLICY_NAME));

            let (policy_attached, policy_document) = match claria_policy {
                Some(p) => {
                    let arn = p.policy_arn().unwrap_or_default();
                    let doc = self.fetch_policy_document(arn).await.unwrap_or(None);
                    (true, doc)
                }
                None => (false, None),
            };

            let mut props = serde_json::json!({
                "user_name": IAM_USER_NAME,
                "user_arn": user_arn,
                "policy_name": IAM_POLICY_NAME,
                "policy_attached": policy_attached,
            });

            if let Some(doc) = policy_document {
                props["policy_document"] = doc;
            }

            Ok(Some(props))
        })
    }

    fn create(&self) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        Box::pin(async {
            // IAM user is managed by bootstrap, not provisioning.
            tracing::info!("IAM user verification recorded (managed by bootstrap)");
            Ok(ResourceResult {
                resource_id: IAM_USER_NAME.to_string(),
                properties: serde_json::json!({ "user_name": IAM_USER_NAME }),
            })
        })
    }

    fn update(&self, resource_id: &str) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        let rid = resource_id.to_string();
        Box::pin(async move {
            tracing::info!("IAM user verification re-recorded (managed by bootstrap)");
            Ok(ResourceResult {
                resource_id: rid,
                properties: serde_json::json!({ "user_name": IAM_USER_NAME }),
            })
        })
    }

    fn delete(&self, _resource_id: &str) -> Bf<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            // IAM user lifecycle is managed by bootstrap, not provisioning.
            tracing::info!("IAM user deletion skipped (managed by bootstrap)");
            Ok(())
        })
    }
}
