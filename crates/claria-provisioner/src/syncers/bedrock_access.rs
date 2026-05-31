use serde_json::json;

use crate::error::ProvisionerError;
use crate::manifest::ResourceSpec;
use crate::syncer::{BoxFuture, ResourceSyncer};

/// Marks that the scoped IAM policy grants the Bedrock + Marketplace + use-case
/// actions Claria needs for chat and model enrollment.
///
/// This carries no behaviour of its own: model-access agreements are now an
/// explicit, user-driven flow in the desktop app (see `claria-bedrock`'s
/// `agreements` module), not something the provisioner accepts on Apply. The
/// spec exists so its `iam_actions` are part of the manifest's aggregated
/// required-actions set, which `IamUserPolicySyncer` compares against the live
/// policy for equality.
pub struct BedrockAccessSyncer {
    spec: ResourceSpec,
}

impl BedrockAccessSyncer {
    pub fn new(spec: ResourceSpec) -> Self {
        Self { spec }
    }
}

impl ResourceSyncer for BedrockAccessSyncer {
    fn spec(&self) -> &ResourceSpec {
        &self.spec
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        // Bedrock is a service — there's no resource to reconcile here. If the
        // IAM policy grants the actions, the user can manage model access in
        // the app's enrollment page.
        Box::pin(async { Ok(Some(json!({"enabled": true}))) })
    }

    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Bedrock access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Bedrock access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }

    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            Err(ProvisionerError::Aws(
                "Bedrock access is a read-only precondition (lifecycle: Data)".into(),
            ))
        })
    }
}
