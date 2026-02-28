use aws_sdk_cloudtrail::Client;
use serde_json::json;

use crate::error::ProvisionerError;
use crate::manifest::{FieldDrift, ResourceSpec};
use crate::syncer::{BoxFuture, ResourceSyncer};

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
                Err(_) => Ok(None),
            }
        })
    }

    fn diff(&self, actual: &serde_json::Value) -> Vec<FieldDrift> {
        let actual_enabled = actual
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let desired_enabled = self
            .spec
            .desired
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if actual_enabled == desired_enabled {
            vec![]
        } else {
            vec![FieldDrift {
                field: "enabled".into(),
                label: "Logging active".into(),
                expected: json!(desired_enabled),
                actual: json!(actual_enabled),
            }]
        }
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            self.client
                .start_logging()
                .name(self.trail_name())
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(e.to_string()))?;

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
                .map_err(|e| ProvisionerError::DeleteFailed(e.to_string()))?;

            tracing::info!(trail = %self.trail_name(), "CloudTrail logging stopped");
            Ok(())
        })
    }
}
