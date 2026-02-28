use aws_sdk_s3::Client;
use serde_json::json;

use crate::error::ProvisionerError;
use crate::manifest::{FieldDrift, ResourceSpec};
use crate::syncer::{BoxFuture, ResourceSyncer};

pub struct S3BucketPolicySyncer {
    spec: ResourceSpec,
    client: Client,
}

impl S3BucketPolicySyncer {
    pub fn new(spec: ResourceSpec, client: Client) -> Self {
        Self { spec, client }
    }

    fn bucket_name(&self) -> &str {
        &self.spec.resource_name
    }

    /// Render the full IAM policy document from the desired spec.
    fn render_policy_document(&self) -> serde_json::Value {
        let statements = self
            .spec
            .desired
            .get("statements")
            .cloned()
            .unwrap_or(json!([]));

        let stmts = statements.as_array().cloned().unwrap_or_default();
        let aws_stmts: Vec<serde_json::Value> = stmts
            .into_iter()
            .map(|s| {
                json!({
                    "Sid": s.get("sid").and_then(|v| v.as_str()).unwrap_or(""),
                    "Effect": s.get("effect").and_then(|v| v.as_str()).unwrap_or("Allow"),
                    "Principal": s.get("principal").map(|p| {
                        if let Some(svc) = p.get("service").and_then(|v| v.as_str()) {
                            json!({"Service": svc})
                        } else {
                            p.clone()
                        }
                    }).unwrap_or(json!("*")),
                    "Action": s.get("action").cloned().unwrap_or(json!("")),
                    "Resource": s.get("resource").cloned().unwrap_or(json!("")),
                    "Condition": s.get("condition").cloned().unwrap_or(json!({})),
                })
            })
            .collect();

        json!({
            "Version": "2012-10-17",
            "Statement": aws_stmts,
        })
    }
}

impl ResourceSyncer for S3BucketPolicySyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self
                .client
                .get_bucket_policy()
                .bucket(self.bucket_name())
                .send()
                .await
            {
                Ok(resp) => {
                    let policy_str = resp.policy().unwrap_or("");
                    let parsed: serde_json::Value =
                        serde_json::from_str(policy_str).unwrap_or(json!(null));
                    Ok(Some(parsed))
                }
                Err(_) => Ok(Some(json!(null))),
            }
        })
    }

    fn diff(&self, actual: &serde_json::Value) -> Vec<FieldDrift> {
        let desired = self.render_policy_document();

        // Compare statement SIDs as a simple diff
        let desired_sids: Vec<&str> = desired
            .get("Statement")
            .and_then(|s| s.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.get("Sid").and_then(|v| v.as_str()))
                    .collect()
            })
            .unwrap_or_default();

        let actual_sids: Vec<&str> = actual
            .get("Statement")
            .and_then(|s| s.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.get("Sid").and_then(|v| v.as_str()))
                    .collect()
            })
            .unwrap_or_default();

        if desired_sids == actual_sids && !actual.is_null() {
            // SIDs match — do a deeper comparison of the full documents
            if desired == *actual {
                return vec![];
            }
        }

        if actual.is_null() || actual_sids != desired_sids {
            vec![FieldDrift {
                field: "statements".into(),
                label: "Policy statements".into(),
                expected: json!(desired_sids),
                actual: json!(actual_sids),
            }]
        } else {
            vec![FieldDrift {
                field: "statements".into(),
                label: "Policy statements".into(),
                expected: desired,
                actual: actual.clone(),
            }]
        }
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            let doc = self.render_policy_document();
            self.client
                .put_bucket_policy()
                .bucket(self.bucket_name())
                .policy(doc.to_string())
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(e.to_string()))?;

            Ok(doc)
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        self.create()
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            let _ = self
                .client
                .delete_bucket_policy()
                .bucket(self.bucket_name())
                .send()
                .await;
            Ok(())
        })
    }
}
