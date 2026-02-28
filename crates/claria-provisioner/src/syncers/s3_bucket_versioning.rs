use aws_sdk_s3::Client;
use serde_json::json;

use crate::error::ProvisionerError;
use crate::manifest::{FieldDrift, ResourceSpec};
use crate::syncer::{BoxFuture, ResourceSyncer};

pub struct S3BucketVersioningSyncer {
    spec: ResourceSpec,
    client: Client,
}

impl S3BucketVersioningSyncer {
    pub fn new(spec: ResourceSpec, client: Client) -> Self {
        Self { spec, client }
    }

    fn bucket_name(&self) -> &str {
        &self.spec.resource_name
    }
}

impl ResourceSyncer for S3BucketVersioningSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self
                .client
                .get_bucket_versioning()
                .bucket(self.bucket_name())
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp
                        .status()
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_default();
                    Ok(Some(json!({"status": status})))
                }
                Err(_) => Ok(None),
            }
        })
    }

    fn diff(&self, actual: &serde_json::Value) -> Vec<FieldDrift> {
        let actual_status = actual
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let desired_status = self
            .spec
            .desired
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("Enabled");

        if actual_status == desired_status {
            vec![]
        } else {
            vec![FieldDrift {
                field: "status".into(),
                label: "Versioning status".into(),
                expected: json!(desired_status),
                actual: json!(actual_status),
            }]
        }
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            self.client
                .put_bucket_versioning()
                .bucket(self.bucket_name())
                .versioning_configuration(
                    aws_sdk_s3::types::VersioningConfiguration::builder()
                        .status(aws_sdk_s3::types::BucketVersioningStatus::Enabled)
                        .build(),
                )
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(e.to_string()))?;

            Ok(json!({"status": "Enabled"}))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        // Idempotent — same as create
        self.create()
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            self.client
                .put_bucket_versioning()
                .bucket(self.bucket_name())
                .versioning_configuration(
                    aws_sdk_s3::types::VersioningConfiguration::builder()
                        .status(aws_sdk_s3::types::BucketVersioningStatus::Suspended)
                        .build(),
                )
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(e.to_string()))?;
            Ok(())
        })
    }
}
