use std::collections::HashSet;

use aws_sdk_iam::Client;
use serde_json::json;

use crate::account_setup::{IAM_POLICY_NAME, IAM_USER_NAME};
use crate::error::ProvisionerError;
use crate::manifest::{FieldDrift, ResourceSpec};
use crate::syncer::{BoxFuture, ResourceSyncer};

pub struct IamUserPolicySyncer {
    spec: ResourceSpec,
    client: Client,
    required_actions: HashSet<String>,
}

impl IamUserPolicySyncer {
    pub fn new(spec: ResourceSpec, client: Client, required_actions: HashSet<String>) -> Self {
        Self {
            spec,
            client,
            required_actions,
        }
    }
}

impl ResourceSyncer for IamUserPolicySyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            // List attached policies for claria-admin
            let resp = self
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
                .map_err(|e| {
                    ProvisionerError::Aws(format!("iam:GetPolicyVersion failed: {e}"))
                })?;

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

    fn diff(&self, actual: &serde_json::Value) -> Vec<FieldDrift> {
        let current_actions: HashSet<String> = actual
            .get("current_actions")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let missing: Vec<String> = self
            .required_actions
            .iter()
            .filter(|a| !current_actions.contains(*a))
            .cloned()
            .collect();

        if missing.is_empty() {
            vec![]
        } else {
            vec![FieldDrift {
                field: "iam_actions".into(),
                label: "IAM permissions".into(),
                expected: json!(self.required_actions.iter().collect::<Vec<_>>()),
                actual: json!(missing),
            }]
        }
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "IAM policy is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "IAM policy is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "IAM policy is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }
}
