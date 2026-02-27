use std::future::Future;
use std::pin::Pin;

use aws_sdk_s3::Client;

use crate::error::ProvisionerError;
use crate::resource::{Resource, ResourceResult};

pub struct S3BucketResource {
    client: Client,
    bucket_name: String,
}

impl S3BucketResource {
    pub fn new(client: Client, bucket_name: String) -> Self {
        Self {
            client,
            bucket_name,
        }
    }
}

impl Resource for S3BucketResource {
    fn resource_type(&self) -> &str {
        "s3_bucket"
    }

    fn current_state(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<serde_json::Value>, ProvisionerError>> + Send + '_>>
    {
        Box::pin(async {
            match self.client.head_bucket().bucket(&self.bucket_name).send().await {
                Ok(_) => Ok(Some(serde_json::json!({
                    "bucket_name": self.bucket_name,
                }))),
                Err(_) => Ok(None),
            }
        })
    }

    fn create(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceResult, ProvisionerError>> + Send + '_>> {
        Box::pin(async {
            self.client
                .create_bucket()
                .bucket(&self.bucket_name)
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(e.to_string()))?;

            self.client
                .put_bucket_versioning()
                .bucket(&self.bucket_name)
                .versioning_configuration(
                    aws_sdk_s3::types::VersioningConfiguration::builder()
                        .status(aws_sdk_s3::types::BucketVersioningStatus::Enabled)
                        .build(),
                )
                .send()
                .await
                .map_err(|e| ProvisionerError::UpdateFailed(e.to_string()))?;

            tracing::info!(bucket = %self.bucket_name, "S3 bucket created with versioning");

            Ok(ResourceResult {
                resource_id: self.bucket_name.clone(),
                properties: serde_json::json!({
                    "bucket_name": self.bucket_name,
                }),
            })
        })
    }

    fn update(
        &self,
        _resource_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceResult, ProvisionerError>> + Send + '_>> {
        let bucket_name = self.bucket_name.clone();
        Box::pin(async move {
            Ok(ResourceResult {
                resource_id: bucket_name.clone(),
                properties: serde_json::json!({
                    "bucket_name": bucket_name,
                }),
            })
        })
    }

    fn delete(
        &self,
        _resource_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ProvisionerError>> + Send + '_>> {
        Box::pin(async {
            self.client
                .delete_bucket()
                .bucket(&self.bucket_name)
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(e.to_string()))?;

            tracing::info!(bucket = %self.bucket_name, "S3 bucket deleted");
            Ok(())
        })
    }
}
