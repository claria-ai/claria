use serde_json::json;

use crate::error::ProvisionerError;
use crate::manifest::{FieldDrift, ResourceSpec};
use crate::syncer::{BoxFuture, ResourceSyncer};

pub struct TranscribeAccessSyncer {
    spec: ResourceSpec,
}

impl TranscribeAccessSyncer {
    pub fn new(spec: ResourceSpec) -> Self {
        Self { spec }
    }
}

impl ResourceSyncer for TranscribeAccessSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        // Transcribe is a service — no resource to check. If the IAM policy
        // grants the actions the user can call Transcribe.
        Box::pin(async { Ok(Some(json!({"enabled": true}))) })
    }

    fn diff(&self, _actual: &serde_json::Value) -> Vec<FieldDrift> {
        vec![]
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Transcribe access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Transcribe access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Transcribe access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }
}
