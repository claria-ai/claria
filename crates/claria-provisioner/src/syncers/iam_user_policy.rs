use std::collections::HashSet;

use aws_sdk_iam::Client;
use serde_json::json;

use crate::{
    account_setup::{self, IAM_POLICY_NAME, IAM_USER_NAME, claria_policy_document},
    error::ProvisionerError,
    manifest::ResourceSpec,
    syncer::{BoxFuture, ResourceSyncer},
};

pub struct IamUserPolicySyncer {
    spec: ResourceSpec,
    client: Client,
    required_actions: HashSet<String>,
    /// system_name and account_id are needed to generate the policy document.
    system_name: String,
    account_id: String,
}

impl IamUserPolicySyncer {
    pub fn new(
        spec: ResourceSpec,
        client: Client,
        required_actions: HashSet<String>,
        system_name: String,
        account_id: String,
    ) -> Self {
        Self {
            spec,
            client,
            required_actions,
            system_name,
            account_id,
        }
    }

    fn policy_arn(&self) -> String {
        format!("arn:aws:iam::{}:policy/{IAM_POLICY_NAME}", self.account_id)
    }
}

impl ResourceSyncer for IamUserPolicySyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            // List attached policies for claria-admin
            let resp = match self
                .client
                .list_attached_user_policies()
                .user_name(IAM_USER_NAME)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    // Access denied or user doesn't exist → can't read
                    let is_no_entity = e
                        .as_service_error()
                        .map(|se| se.is_no_such_entity_exception())
                        .unwrap_or(false);
                    let is_access_denied = e
                        .as_service_error()
                        .map(|se| se.meta().code() == Some("AccessDenied"))
                        .unwrap_or(false);
                    if is_no_entity || is_access_denied {
                        return Ok(None);
                    }
                    return Err(ProvisionerError::Aws(format!(
                        "iam:ListAttachedUserPolicies failed: {e}"
                    )));
                }
            };

            let claria_policy = resp
                .attached_policies()
                .iter()
                .find(|p| p.policy_name() == Some(IAM_POLICY_NAME));

            let Some(policy) = claria_policy else {
                return Ok(None);
            };

            let policy_arn = policy.policy_arn().unwrap_or_default();

            // Get default version
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

            // Fetch actual document
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

            let decoded = percent_encoding::percent_decode_str(doc_str)
                .decode_utf8()
                .unwrap_or_default();
            let parsed: serde_json::Value =
                serde_json::from_str(&decoded).unwrap_or(serde_json::Value::Null);

            if parsed.is_null() {
                return Ok(None);
            }

            // Extract allowed actions from the policy document
            let mut current_actions: HashSet<String> = HashSet::new();
            if let Some(statements) = parsed.get("Statement").and_then(|s| s.as_array()) {
                for stmt in statements {
                    if stmt.get("Effect").and_then(|e| e.as_str()) != Some("Allow") {
                        continue;
                    }
                    match stmt.get("Action") {
                        Some(serde_json::Value::String(a)) => {
                            current_actions.insert(a.clone());
                        }
                        Some(serde_json::Value::Array(arr)) => {
                            for a in arr {
                                if let Some(s) = a.as_str() {
                                    current_actions.insert(s.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            Ok(Some(json!({
                "policy_attached": true,
                "policy_document": parsed,
                "current_actions": current_actions.into_iter().collect::<Vec<_>>(),
            })))
        })
    }

    fn desired_state(&self) -> serde_json::Value {
        let mut actions: Vec<&String> = self.required_actions.iter().collect();
        actions.sort();
        json!({"actions": actions})
    }

    fn current_state(&self, actual: &serde_json::Value) -> serde_json::Value {
        let mut actions: Vec<String> = actual
            .get("current_actions")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        actions.sort();
        json!({"actions": actions})
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            // Create the policy and attach it to the user.
            let policy_arn = account_setup::create_policy(
                &self.client,
                &self.system_name,
                &Some(self.account_id.clone()),
            )
            .await?;

            account_setup::attach_policy(&self.client, &policy_arn).await?;

            Ok(json!({"policy_attached": true}))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            // Update the policy document to add/remove actions.
            let document = claria_policy_document(&self.system_name, &self.account_id);
            account_setup::update_policy_document(&self.client, &self.policy_arn(), &document)
                .await?;

            Ok(json!({"policy_attached": true}))
        })
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            let policy_arn = self.policy_arn();

            // Detach from user first.
            let _ = self
                .client
                .detach_user_policy()
                .user_name(IAM_USER_NAME)
                .policy_arn(&policy_arn)
                .send()
                .await;

            // Delete all non-default versions before deleting the policy.
            if let Ok(versions) = self
                .client
                .list_policy_versions()
                .policy_arn(&policy_arn)
                .send()
                .await
            {
                for v in versions
                    .versions()
                    .iter()
                    .filter(|v| !v.is_default_version())
                {
                    if let Some(vid) = v.version_id() {
                        let _ = self
                            .client
                            .delete_policy_version()
                            .policy_arn(&policy_arn)
                            .version_id(vid)
                            .send()
                            .await;
                    }
                }
            }

            self.client
                .delete_policy()
                .policy_arn(&policy_arn)
                .send()
                .await
                .map_err(|e| ProvisionerError::Aws(format!("iam:DeletePolicy failed: {e}")))?;

            tracing::info!(policy_arn = %policy_arn, "deleted IAM policy");
            Ok(())
        })
    }
}
