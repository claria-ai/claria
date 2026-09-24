use aws_sdk_s3::{Client, error::ProvideErrorMetadata};
use aws_smithy_types::error::display::DisplayErrorContext;
use serde_json::json;

use crate::{
    error::ProvisionerError,
    manifest::ResourceSpec,
    syncer::{BoxFuture, ResourceSyncer},
    syncers::read_failed,
};

pub struct S3BucketEncryptionSyncer {
    spec: ResourceSpec,
    client: Client,
}

impl S3BucketEncryptionSyncer {
    pub fn new(spec: ResourceSpec, client: Client) -> Self {
        Self { spec, client }
    }

    fn bucket_name(&self) -> &str {
        &self.spec.resource_name
    }
}

impl ResourceSyncer for S3BucketEncryptionSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self
                .client
                .get_bucket_encryption()
                .bucket(self.bucket_name())
                .send()
                .await
            {
                Ok(resp) => {
                    let algo = resp
                        .server_side_encryption_configuration()
                        .and_then(|config| config.rules().first())
                        .and_then(|rule| rule.apply_server_side_encryption_by_default())
                        .map(|default| default.sse_algorithm().as_str().to_string());
                    Ok(Some(json!({"sse_algorithm": algo})))
                }
                // A bucket with no encryption rule answers with this code —
                // the one case where "no algorithm" is the truth rather than
                // a read that did not happen.
                Err(e) if e.code() == Some("ServerSideEncryptionConfigurationNotFoundError") => {
                    Ok(Some(json!({"sse_algorithm": null})))
                }
                Err(e) if e.code() == Some("NoSuchBucket") => Ok(None),
                // Anything else is unread. Reporting it as unencrypted would
                // show the clinician drift that may not exist, and then fail
                // the write that tried to fix it.
                Err(e) => Err(read_failed("s3:GetEncryptionConfiguration", &e)),
            }
        })
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            self.client
                .put_bucket_encryption()
                .bucket(self.bucket_name())
                .server_side_encryption_configuration(
                    aws_sdk_s3::types::ServerSideEncryptionConfiguration::builder()
                        .rules(
                            aws_sdk_s3::types::ServerSideEncryptionRule::builder()
                                .apply_server_side_encryption_by_default(
                                    aws_sdk_s3::types::ServerSideEncryptionByDefault::builder()
                                        .sse_algorithm(
                                            aws_sdk_s3::types::ServerSideEncryption::Aes256,
                                        )
                                        .build()
                                        .map_err(|e| {
                                            ProvisionerError::CreateFailed(
                                                DisplayErrorContext(&e).to_string(),
                                            )
                                        })?,
                                )
                                .build(),
                        )
                        .build()
                        .map_err(|e| {
                            ProvisionerError::CreateFailed(DisplayErrorContext(&e).to_string())
                        })?,
                )
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(DisplayErrorContext(&e).to_string()))?;

            Ok(json!({"sse_algorithm": "AES256"}))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        self.create()
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            // Deleting encryption config is generally not advisable; skip.
            Ok(())
        })
    }
}
