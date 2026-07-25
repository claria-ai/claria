use aws_sdk_s3::Client;
use aws_smithy_types::error::display::DisplayErrorContext;
use serde_json::json;

use crate::error::ProvisionerError;
use crate::manifest::ResourceSpec;
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
                .map_err(|e| ProvisionerError::CreateFailed(DisplayErrorContext(&e).to_string()))?;

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
            // Delete every object before the bucket itself.
            let mut pages = self
                .client
                .list_objects_v2()
                .bucket(self.bucket_name())
                .into_paginator()
                .send();

            while let Some(page) = pages.next().await {
                let page = page
                    .map_err(|e| ProvisionerError::DeleteFailed(DisplayErrorContext(&e).to_string()))?;

                for obj in page.contents() {
                    if let Some(key) = obj.key() {
                        self.client
                            .delete_object()
                            .bucket(self.bucket_name())
                            .key(key)
                            .send()
                            .await
                            .map_err(|e| {
                                ProvisionerError::DeleteFailed(
                                    DisplayErrorContext(&e).to_string(),
                                )
                            })?;
                    }
                }
            }

            self.client
                .delete_bucket()
                .bucket(self.bucket_name())
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(DisplayErrorContext(&e).to_string()))?;

            tracing::info!(bucket = %self.bucket_name(), "S3 bucket deleted");
            Ok(())
        })
    }
}
