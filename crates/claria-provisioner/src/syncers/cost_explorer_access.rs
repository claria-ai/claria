use serde_json::json;

use crate::error::ProvisionerError;
use crate::manifest::ResourceSpec;
use crate::syncer::{BoxFuture, ResourceSyncer};

pub struct CostExplorerAccessSyncer {
    spec: ResourceSpec,
}

impl CostExplorerAccessSyncer {
    pub fn new(spec: ResourceSpec) -> Self {
        Self { spec }
    }
}

impl ResourceSyncer for CostExplorerAccessSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        // Cost Explorer is a service — no resource to check. If the IAM policy
        // grants the action the user can call Cost Explorer.
        Box::pin(async { Ok(Some(json!({"enabled": true}))) })
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
