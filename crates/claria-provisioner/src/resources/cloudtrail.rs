use std::future::Future;
use std::pin::Pin;

use aws_sdk_cloudtrail::Client;

use crate::error::ProvisionerError;
use crate::resource::{Resource, ResourceResult};

type Bf<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct CloudTrailResource {
    client: Client,
    trail_name: String,
    s3_bucket: String,
}

impl CloudTrailResource {
    pub fn new(client: Client, trail_name: String, s3_bucket: String) -> Self {
        Self {
            client,
            trail_name,
            s3_bucket,
        }
    }
}

impl Resource for CloudTrailResource {
    fn resource_type(&self) -> &str {
        "cloudtrail_trail"
    }

    fn current_state(&self) -> Bf<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self.client.get_trail().name(&self.trail_name).send().await {
                Ok(resp) => {
                    let trail = resp.trail();
                    Ok(Some(serde_json::json!({
                        "trail_name": self.trail_name,
                        "trail_arn": trail.map(|t| t.trail_arn().unwrap_or_default()),
                        "s3_bucket": trail.map(|t| t.s3_bucket_name().unwrap_or_default()),
                    })))
                }
                Err(_) => Ok(None),
            }
        })
    }

    fn create(&self) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        Box::pin(async {
            let result = self
                .client
                .create_trail()
                .name(&self.trail_name)
                .s3_bucket_name(&self.s3_bucket)
                .is_multi_region_trail(false)
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(e.to_string()))?;

            let trail_arn = result.trail_arn().unwrap_or_default().to_string();

            self.client
                .start_logging()
                .name(&self.trail_name)
                .send()
                .await
                .map_err(|e| ProvisionerError::UpdateFailed(e.to_string()))?;

            tracing::info!(
                trail_name = %self.trail_name,
                trail_arn = %trail_arn,
                "CloudTrail trail created and logging started"
            );

            Ok(ResourceResult {
                resource_id: trail_arn.clone(),
                properties: serde_json::json!({
                    "trail_name": self.trail_name,
                    "trail_arn": trail_arn,
                    "s3_bucket": self.s3_bucket,
                }),
            })
        })
    }

    fn update(&self, resource_id: &str) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        let rid = resource_id.to_string();
        let trail_name = self.trail_name.clone();
        let s3_bucket = self.s3_bucket.clone();
        Box::pin(async move {
            Ok(ResourceResult {
                resource_id: rid,
                properties: serde_json::json!({
                    "trail_name": trail_name,
                    "s3_bucket": s3_bucket,
                }),
            })
        })
    }

    fn delete(&self, _resource_id: &str) -> Bf<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            self.client
                .delete_trail()
                .name(&self.trail_name)
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(e.to_string()))?;
            tracing::info!(trail_name = %self.trail_name, "CloudTrail trail deleted");
            Ok(())
        })
    }
}
