use aws_sdk_cloudtrail::{Client, error::ProvideErrorMetadata};
use aws_smithy_types::error::display::DisplayErrorContext;
use serde_json::json;

use crate::{
    error::ProvisionerError,
    manifest::ResourceSpec,
    syncer::{BoxFuture, ResourceSyncer},
    syncers::read_failed,
};

pub struct CloudTrailTrailSyncer {
    spec: ResourceSpec,
    client: Client,
}

impl CloudTrailTrailSyncer {
    pub fn new(spec: ResourceSpec, client: Client) -> Self {
        Self { spec, client }
    }

    fn trail_name(&self) -> &str {
        &self.spec.resource_name
    }
}

impl ResourceSyncer for CloudTrailTrailSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            let resp = match self.client.get_trail().name(self.trail_name()).send().await {
                Ok(resp) => resp,
                // The trail is genuinely not there — the one absence.
                Err(e) if e.code() == Some("TrailNotFoundException") => return Ok(None),
                // Everything else is a read that did not happen. The audit
                // trail is a HIPAA requirement, so "I could not check" must
                // never arrive as "it needs creating" and then as a create
                // that fails.
                Err(e) => return Err(read_failed("cloudtrail:GetTrail", &e)),
            };

            let trail = resp.trail();
            let s3_bucket = trail.and_then(|t| t.s3_bucket_name()).unwrap_or_default();
            let s3_key_prefix = trail.and_then(|t| t.s3_key_prefix()).unwrap_or_default();
            let is_multi_region = trail
                .and_then(|t| t.is_multi_region_trail())
                .unwrap_or(false);

            Ok(Some(json!({
                "s3_bucket": s3_bucket,
                "s3_key_prefix": s3_key_prefix,
                "is_multi_region": is_multi_region,
            })))
        })
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            let s3_bucket = self
                .spec
                .desired
                .get("s3_bucket")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let s3_key_prefix = self
                .spec
                .desired
                .get("s3_key_prefix")
                .and_then(|v| v.as_str())
                .unwrap_or("_cloudtrail");

            let result = self
                .client
                .create_trail()
                .name(self.trail_name())
                .s3_bucket_name(s3_bucket)
                .s3_key_prefix(s3_key_prefix)
                .is_multi_region_trail(false)
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(DisplayErrorContext(&e).to_string()))?;

            let trail_arn = result.trail_arn().unwrap_or_default().to_string();
            tracing::info!(trail = %self.trail_name(), arn = %trail_arn, "CloudTrail trail created");

            Ok(json!({
                "s3_bucket": s3_bucket,
                "s3_key_prefix": s3_key_prefix,
                "is_multi_region": false,
                "trail_arn": trail_arn,
            }))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        // Trail update is effectively recreate
        self.create()
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            // Stop logging first (ignore errors)
            let _ = self
                .client
                .stop_logging()
                .name(self.trail_name())
                .send()
                .await;

            self.client
                .delete_trail()
                .name(self.trail_name())
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(DisplayErrorContext(&e).to_string()))?;

            tracing::info!(trail = %self.trail_name(), "CloudTrail trail deleted");
            Ok(())
        })
    }
}
