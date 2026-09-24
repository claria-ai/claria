use aws_sdk_costexplorer::{Client, types::DateInterval};
use serde_json::json;

use crate::{
    error::ProvisionerError,
    manifest::{GRANTED_BY_POLICY, ResourceSpec, VERIFIED},
    syncer::{BoxFuture, ResourceSyncer},
    syncers::read_failed,
};

pub struct CostExplorerAccessSyncer {
    spec: ResourceSpec,
    client: Client,
}

impl CostExplorerAccessSyncer {
    pub fn new(spec: ResourceSpec, config: &aws_config::SdkConfig) -> Self {
        Self {
            spec,
            client: Client::new(config),
        }
    }

    /// Whether this manifest asked for the access to be proved.
    ///
    /// See [`crate::manifest::Manifest::with_cost_explorer_probe`] for why it
    /// is not simply always on.
    fn probes(&self) -> bool {
        self.spec.desired.get("access").and_then(|v| v.as_str()) == Some(VERIFIED)
    }
}

impl ResourceSyncer for CostExplorerAccessSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            if !self.probes() {
                // Not a check, and no longer dressed as one. The policy grants
                // the action; whether the account can use it is proved the
                // first time the Cost page asks.
                return Ok(Some(json!({"access": GRANTED_BY_POLICY})));
            }

            // The smallest question Cost Explorer answers: one day, one
            // metric, no grouping. AWS bills $0.01 for it either way, which
            // is why this runs only for an operator already using the API.
            let today = jiff::Zoned::now().date();
            let yesterday = today.yesterday().unwrap_or(today);

            match self
                .client
                .get_cost_and_usage()
                .time_period(
                    DateInterval::builder()
                        .start(yesterday.to_string())
                        .end(today.to_string())
                        .build()
                        .map_err(|e| {
                            ProvisionerError::Aws(format!("could not build a date range: {e}"))
                        })?,
                )
                .granularity(aws_sdk_costexplorer::types::Granularity::Daily)
                .metrics("UnblendedCost")
                .send()
                .await
            {
                Ok(_) => Ok(Some(json!({"access": VERIFIED}))),
                Err(e) => Err(read_failed("ce:GetCostAndUsage", &e)),
            }
        })
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Cost Explorer access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Cost Explorer access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Cost Explorer access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }
}
