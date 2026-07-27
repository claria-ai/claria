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

    /// Whether the bucket is there, distinguishing "gone" from "couldn't tell".
    ///
    /// `HeadBucket` answers 404 with an empty body, so the status line is the
    /// reliable signal and the typed `NotFound` is the backstop.
    async fn bucket_exists(&self) -> Result<bool, ProvisionerError> {
        match self
            .client
            .head_bucket()
            .bucket(self.bucket_name())
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => {
                let status_404 = e.raw_response().is_some_and(|r| r.status().as_u16() == 404);
                let err = e.into_service_error();
                if status_404 || err.is_not_found() {
                    Ok(false)
                } else {
                    Err(ProvisionerError::DeleteFailed(
                        DisplayErrorContext(&err).to_string(),
                    ))
                }
            }
        }
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
            // A re-run after a destroy that got as far as the bucket itself
            // has nothing left to do. Only a genuine 404 counts — any other
            // failure here would otherwise report the bucket as destroyed.
            if !self.bucket_exists().await? {
                tracing::info!(
                    bucket = %self.bucket_name(),
                    "S3 bucket already absent, nothing to destroy"
                );
                return Ok(());
            }

            // Empty the bucket before deleting it. Claria's buckets have
            // versioning on, where deleting an object writes a delete marker
            // and keeps the data, so every version has to go explicitly.
            let purged = claria_storage::objects::delete_all_object_versions(
                &self.client,
                self.bucket_name(),
                "",
            )
            .await
            .map_err(|e| ProvisionerError::DeleteFailed(e.to_string()))?;

            tracing::info!(
                bucket = %self.bucket_name(),
                versions = purged,
                "emptied S3 bucket"
            );

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
