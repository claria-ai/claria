use aws_sdk_s3::Client;
use serde_json::json;

use crate::error::{format_err_chain, ProvisionerError};
use crate::manifest::{FieldDrift, ResourceSpec};
use crate::syncer::{BoxFuture, ResourceSyncer};

pub struct S3BucketSyncer {
    spec: ResourceSpec,
    client: Client,
}

impl S3BucketSyncer {
    pub fn new(spec: ResourceSpec, client: Client) -> Self {
        Self { spec, client }
    }

    fn bucket_name(&self) -> &str {
        &self.spec.resource_name
    }

    fn region(&self) -> &str {
        self.spec
            .desired
            .get("region")
            .and_then(|v| v.as_str())
            .unwrap_or("us-east-1")
    }
}

impl ResourceSyncer for S3BucketSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self
                .client
                .head_bucket()
                .bucket(self.bucket_name())
                .send()
                .await
            {
                Ok(_) => Ok(Some(json!({"region": self.region()}))),
                Err(_) => Ok(None),
            }
        })
    }

    fn diff(&self, _actual: &serde_json::Value) -> Vec<FieldDrift> {
        // Binary: bucket exists or not. Region can't change after creation.
        vec![]
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            let mut builder = self.client.create_bucket().bucket(self.bucket_name());

            if self.region() != "us-east-1" {
                builder = builder.create_bucket_configuration(
                    aws_sdk_s3::types::CreateBucketConfiguration::builder()
                        .location_constraint(aws_sdk_s3::types::BucketLocationConstraint::from(
                            self.region(),
                        ))
                        .build(),
                );
            }

            builder
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(format_err_chain(&e)))?;

            tracing::info!(bucket = %self.bucket_name(), "S3 bucket created");

            Ok(json!({"region": self.region()}))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        // Bucket exists — nothing to modify at this level
        Box::pin(async { Ok(json!({"region": self.region()})) })
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            // Paginated delete all objects first
            let mut continuation_token = None;
            loop {
                let mut list = self.client.list_objects_v2().bucket(self.bucket_name());
                if let Some(token) = &continuation_token {
                    list = list.continuation_token(token);
                }
                let resp = list
                    .send()
                    .await
                    .map_err(|e| ProvisionerError::DeleteFailed(format_err_chain(&e)))?;

                for obj in resp.contents() {
                    if let Some(key) = obj.key() {
                        self.client
                            .delete_object()
                            .bucket(self.bucket_name())
                            .key(key)
                            .send()
                            .await
                            .map_err(|e| ProvisionerError::DeleteFailed(format_err_chain(&e)))?;
                    }
                }

                if resp.is_truncated() == Some(true) {
                    continuation_token = resp.next_continuation_token().map(String::from);
                } else {
                    break;
                }
            }

            self.client
                .delete_bucket()
                .bucket(self.bucket_name())
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(format_err_chain(&e)))?;

            tracing::info!(bucket = %self.bucket_name(), "S3 bucket deleted");
            Ok(())
        })
    }
}
