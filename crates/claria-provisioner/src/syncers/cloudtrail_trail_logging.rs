use aws_sdk_cloudtrail::{Client, error::ProvideErrorMetadata};
use aws_smithy_types::error::display::DisplayErrorContext;
use serde_json::json;

use crate::{
    error::ProvisionerError,
    manifest::ResourceSpec,
    syncer::{BoxFuture, ResourceSyncer},
    syncers::read_failed,
};

pub struct CloudTrailTrailLoggingSyncer {
    spec: ResourceSpec,
    client: Client,
}

impl CloudTrailTrailLoggingSyncer {
    pub fn new(spec: ResourceSpec, client: Client) -> Self {
        Self { spec, client }
    }

    fn trail_name(&self) -> &str {
        &self.spec.resource_name
    }
}

impl ResourceSyncer for CloudTrailTrailLoggingSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self
                .client
                .get_trail_status()
                .name(self.trail_name())
                .send()
                .await
            {
                Ok(status) => {
                    let is_logging = status.is_logging().unwrap_or(false);
                    Ok(Some(json!({"enabled": is_logging})))
                }
                // No trail, so no logging status to report.
                Err(e) if e.code() == Some("TrailNotFoundException") => Ok(None),
                Err(e) => Err(read_failed("cloudtrail:GetTrailStatus", &e)),
            }
        })
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            self.client
                .start_logging()
                .name(self.trail_name())
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(DisplayErrorContext(&e).to_string()))?;

            tracing::info!(trail = %self.trail_name(), "CloudTrail logging started");
            Ok(json!({"enabled": true}))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        self.create()
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            self.client
                .stop_logging()
                .name(self.trail_name())
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(DisplayErrorContext(&e).to_string()))?;

            tracing::info!(trail = %self.trail_name(), "CloudTrail logging stopped");
            Ok(())
        })
    }
}
