use aws_sdk_s3::Client;
use serde_json::json;

use crate::error::{format_err_chain, ProvisionerError};
use crate::manifest::ResourceSpec;
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

    fn desired_state(&self) -> serde_json::Value {
        self.render_policy_document()
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
                .map_err(|e| ProvisionerError::CreateFailed(format_err_chain(&e)))?;

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
