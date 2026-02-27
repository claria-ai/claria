use std::future::Future;
use std::pin::Pin;

use aws_sdk_s3::Client;

use crate::error::ProvisionerError;
use crate::resource::{Resource, ResourceResult};

pub struct S3BucketResource {
    client: Client,
    bucket_name: String,
    region: String,
    account_id: String,
}

impl S3BucketResource {
    pub fn new(client: Client, bucket_name: String, region: String, account_id: String) -> Self {
        Self {
            client,
            bucket_name,
            region,
            account_id,
        }
    }

    /// Check if versioning is enabled.
    async fn check_versioning(&self) -> Option<String> {
        match self
            .client
            .get_bucket_versioning()
            .bucket(&self.bucket_name)
            .send()
            .await
        {
            Ok(resp) => resp
                .status()
                .map(|s| s.as_str().to_string()),
            Err(_) => None,
        }
    }

    /// Check server-side encryption configuration.
    async fn check_encryption(&self) -> Option<String> {
        match self
            .client
            .get_bucket_encryption()
            .bucket(&self.bucket_name)
            .send()
            .await
        {
            Ok(resp) => resp
                .server_side_encryption_configuration()
                .and_then(|config| config.rules().first())
                .and_then(|rule| rule.apply_server_side_encryption_by_default())
                .map(|default| {
                    default
                        .sse_algorithm()
                        .as_str()
                        .to_string()
                }),
            Err(_) => None,
        }
    }

    /// Check public access block settings.
    async fn check_public_access_block(&self) -> Option<serde_json::Value> {
        match self
            .client
            .get_public_access_block()
            .bucket(&self.bucket_name)
            .send()
            .await
        {
            Ok(resp) => resp.public_access_block_configuration().map(|config| {
                serde_json::json!({
                    "block_public_acls": config.block_public_acls(),
                    "ignore_public_acls": config.ignore_public_acls(),
                    "block_public_policy": config.block_public_policy(),
                    "restrict_public_buckets": config.restrict_public_buckets(),
                })
            }),
            Err(_) => None,
        }
    }

    /// Apply all hardening settings to the bucket.
    async fn apply_hardening(&self) -> Result<(), ProvisionerError> {
        // Encryption: AES256
        self.client
            .put_bucket_encryption()
            .bucket(&self.bucket_name)
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
                                    .map_err(|e| ProvisionerError::UpdateFailed(e.to_string()))?,
                            )
                            .build(),
                    )
                    .build()
                    .map_err(|e| ProvisionerError::UpdateFailed(e.to_string()))?,
            )
            .send()
            .await
            .map_err(|e| ProvisionerError::UpdateFailed(e.to_string()))?;

        // Versioning: Enabled
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

        // Public access block: all four flags
        self.client
            .put_public_access_block()
            .bucket(&self.bucket_name)
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
            .map_err(|e| ProvisionerError::UpdateFailed(e.to_string()))?;

        // Bucket policy: allow CloudTrail to write logs
        let policy = serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [
                {
                    "Sid": "AWSCloudTrailAclCheck",
                    "Effect": "Allow",
                    "Principal": { "Service": "cloudtrail.amazonaws.com" },
                    "Action": "s3:GetBucketAcl",
                    "Resource": format!("arn:aws:s3:::{}", self.bucket_name),
                    "Condition": {
                        "StringEquals": {
                            "AWS:SourceAccount": self.account_id
                        }
                    }
                },
                {
                    "Sid": "AWSCloudTrailWrite",
                    "Effect": "Allow",
                    "Principal": { "Service": "cloudtrail.amazonaws.com" },
                    "Action": "s3:PutObject",
                    "Resource": format!(
                        "arn:aws:s3:::{}/_cloudtrail/AWSLogs/{}/*",
                        self.bucket_name, self.account_id
                    ),
                    "Condition": {
                        "StringEquals": {
                            "s3:x-amz-acl": "bucket-owner-full-control",
                            "AWS:SourceAccount": self.account_id
                        }
                    }
                }
            ]
        });

        self.client
            .put_bucket_policy()
            .bucket(&self.bucket_name)
            .policy(policy.to_string())
            .send()
            .await
            .map_err(|e| ProvisionerError::UpdateFailed(e.to_string()))?;

        Ok(())
    }

    fn build_properties(
        &self,
        versioning: &Option<String>,
        encryption: &Option<String>,
        public_access_block: &Option<serde_json::Value>,
    ) -> serde_json::Value {
        serde_json::json!({
            "bucket_name": self.bucket_name,
            "versioning": versioning,
            "encryption": encryption,
            "public_access_block": public_access_block,
        })
    }
}

impl Resource for S3BucketResource {
    fn resource_type(&self) -> &str {
        "s3_bucket"
    }

    fn expected_id(&self) -> Option<&str> {
        Some(&self.bucket_name)
    }

    fn current_state(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<serde_json::Value>, ProvisionerError>> + Send + '_>>
    {
        Box::pin(async {
            // Check if bucket exists
            match self
                .client
                .head_bucket()
                .bucket(&self.bucket_name)
                .send()
                .await
            {
                Ok(_) => {}
                Err(_) => return Ok(None),
            }

            // Bucket exists — gather hardening details
            let versioning = self.check_versioning().await;
            let encryption = self.check_encryption().await;
            let public_access_block = self.check_public_access_block().await;

            Ok(Some(self.build_properties(
                &versioning,
                &encryption,
                &public_access_block,
            )))
        })
    }

    fn create(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceResult, ProvisionerError>> + Send + '_>> {
        Box::pin(async {
            // Create bucket with location constraint for non-us-east-1 regions
            let mut builder = self
                .client
                .create_bucket()
                .bucket(&self.bucket_name);

            if self.region != "us-east-1" {
                builder = builder.create_bucket_configuration(
                    aws_sdk_s3::types::CreateBucketConfiguration::builder()
                        .location_constraint(
                            aws_sdk_s3::types::BucketLocationConstraint::from(
                                self.region.as_str(),
                            ),
                        )
                        .build(),
                );
            }

            builder
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(e.to_string()))?;

            // Apply all hardening
            self.apply_hardening().await?;

            tracing::info!(
                bucket = %self.bucket_name,
                "S3 bucket created with encryption, versioning, and public access block"
            );

            let versioning = Some("Enabled".to_string());
            let encryption = Some("AES256".to_string());
            let public_access_block = Some(serde_json::json!({
                "block_public_acls": true,
                "ignore_public_acls": true,
                "block_public_policy": true,
                "restrict_public_buckets": true,
            }));

            Ok(ResourceResult {
                resource_id: self.bucket_name.clone(),
                properties: self.build_properties(&versioning, &encryption, &public_access_block),
            })
        })
    }

    fn update(
        &self,
        _resource_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceResult, ProvisionerError>> + Send + '_>> {
        Box::pin(async {
            // Re-apply all hardening — idempotent
            self.apply_hardening().await?;

            tracing::info!(
                bucket = %self.bucket_name,
                "S3 bucket hardening applied"
            );

            let versioning = Some("Enabled".to_string());
            let encryption = Some("AES256".to_string());
            let public_access_block = Some(serde_json::json!({
                "block_public_acls": true,
                "ignore_public_acls": true,
                "block_public_policy": true,
                "restrict_public_buckets": true,
            }));

            Ok(ResourceResult {
                resource_id: self.bucket_name.clone(),
                properties: self.build_properties(&versioning, &encryption, &public_access_block),
            })
        })
    }

    fn delete(
        &self,
        _resource_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ProvisionerError>> + Send + '_>> {
        Box::pin(async {
            // List and delete all objects first
            let mut continuation_token = None;
            loop {
                let mut list = self
                    .client
                    .list_objects_v2()
                    .bucket(&self.bucket_name);
                if let Some(token) = &continuation_token {
                    list = list.continuation_token(token);
                }
                let resp = list
                    .send()
                    .await
                    .map_err(|e| ProvisionerError::DeleteFailed(e.to_string()))?;

                for obj in resp.contents() {
                    if let Some(key) = obj.key() {
                        self.client
                            .delete_object()
                            .bucket(&self.bucket_name)
                            .key(key)
                            .send()
                            .await
                            .map_err(|e| {
                                ProvisionerError::DeleteFailed(e.to_string())
                            })?;
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
                .bucket(&self.bucket_name)
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(e.to_string()))?;

            tracing::info!(bucket = %self.bucket_name, "S3 bucket deleted");
            Ok(())
        })
    }
}
