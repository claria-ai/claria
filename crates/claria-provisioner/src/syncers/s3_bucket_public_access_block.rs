use aws_sdk_s3::Client;
use serde_json::json;

use crate::error::{format_err_chain, ProvisionerError};
use crate::manifest::{FieldDrift, ResourceSpec};
use crate::syncer::{BoxFuture, ResourceSyncer};

pub struct S3BucketPublicAccessBlockSyncer {
    spec: ResourceSpec,
    client: Client,
}

impl S3BucketPublicAccessBlockSyncer {
    pub fn new(spec: ResourceSpec, client: Client) -> Self {
        Self { spec, client }
    }

    fn bucket_name(&self) -> &str {
        &self.spec.resource_name
    }
}

impl ResourceSyncer for S3BucketPublicAccessBlockSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self
                .client
                .get_public_access_block()
                .bucket(self.bucket_name())
                .send()
                .await
            {
                Ok(resp) => {
                    let config = resp.public_access_block_configuration();
                    Ok(Some(json!({
                        "block_public_acls": config.and_then(|c| c.block_public_acls()).unwrap_or(false),
                        "ignore_public_acls": config.and_then(|c| c.ignore_public_acls()).unwrap_or(false),
                        "block_public_policy": config.and_then(|c| c.block_public_policy()).unwrap_or(false),
                        "restrict_public_buckets": config.and_then(|c| c.restrict_public_buckets()).unwrap_or(false),
                    })))
                }
                Err(_) => Ok(Some(json!({
                    "block_public_acls": false,
                    "ignore_public_acls": false,
                    "block_public_policy": false,
                    "restrict_public_buckets": false,
                }))),
            }
        })
    }

    fn diff(&self, actual: &serde_json::Value) -> Vec<FieldDrift> {
        let fields = [
            ("block_public_acls", "Block public ACLs"),
            ("ignore_public_acls", "Ignore public ACLs"),
            ("block_public_policy", "Block public policy"),
            ("restrict_public_buckets", "Restrict public buckets"),
        ];

        let mut drifts = Vec::new();
        for (field, label) in &fields {
            let expected = self
                .spec
                .desired
                .get(*field)
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let actual_val = actual
                .get(*field)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if expected != actual_val {
                drifts.push(FieldDrift {
                    field: field.to_string(),
                    label: label.to_string(),
                    expected: json!(expected),
                    actual: json!(actual_val),
                });
            }
        }
        drifts
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            self.client
                .put_public_access_block()
                .bucket(self.bucket_name())
                .public_access_block_configuration(
                    aws_sdk_s3::types::PublicAccessBlockConfiguration::builder()
                        .block_public_acls(true)
                        .ignore_public_acls(true)
                        .block_public_policy(true)
                        .restrict_public_buckets(true)
                        .build(),
                )
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(format_err_chain(&e)))?;

            Ok(json!({
                "block_public_acls": true,
                "ignore_public_acls": true,
                "block_public_policy": true,
                "restrict_public_buckets": true,
            }))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        self.create()
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            let _ = self
                .client
                .delete_public_access_block()
                .bucket(self.bucket_name())
                .send()
                .await;
            Ok(())
        })
    }
}
