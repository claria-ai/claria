//! claria-provisioner
//!
//! Statically compiled IaC engine for provisioning and managing
//! Claria's AWS infrastructure. Library consumed by the Tauri desktop app.

pub mod drift;
pub mod error;
pub mod plan;
pub mod resource;
pub mod resources;
pub mod state;
pub mod sync;

use aws_sdk_s3::Client as S3Client;

use claria_core::s3_keys;
use claria_storage::state::{load_state, save_state};

use crate::error::ProvisionerError;
use crate::resource::Resource;
use crate::state::ProvisionerState;

/// Load the provisioner state from S3.
pub async fn load_provisioner_state(
    s3: &S3Client,
    bucket: &str,
) -> Result<ProvisionerState, ProvisionerError> {
    match load_state::<ProvisionerState>(s3, bucket, s3_keys::PROVISIONER_STATE).await {
        Ok((state, _etag)) => Ok(state),
        Err(claria_storage::error::StorageError::NotFound { .. }) => {
            Ok(ProvisionerState::default())
        }
        Err(e) => Err(e.into()),
    }
}

/// Save the provisioner state to S3.
pub async fn save_provisioner_state(
    s3: &S3Client,
    bucket: &str,
    state: &ProvisionerState,
) -> Result<(), ProvisionerError> {
    save_state(s3, bucket, s3_keys::PROVISIONER_STATE, state).await?;
    Ok(())
}

/// Full provisioning: detect drift, plan, execute, save state.
pub async fn provision(
    s3: &S3Client,
    bucket: &str,
    resources: &[Box<dyn Resource>],
) -> Result<(), ProvisionerError> {
    let mut state = load_provisioner_state(s3, bucket).await?;

    let plan = drift::detect_drift(&state, resources).await?;

    if plan.has_changes() {
        tracing::info!(
            actions = plan.actions.len(),
            "executing provisioning plan"
        );
        sync::execute_plan(&plan, resources, &mut state).await?;
        save_provisioner_state(s3, bucket, &state).await?;
    } else {
        tracing::info!("all resources in sync, no changes needed");
    }

    Ok(())
}

/// Destroy all managed resources.
pub async fn destroy(
    s3: &S3Client,
    bucket: &str,
    resources: &[Box<dyn Resource>],
) -> Result<(), ProvisionerError> {
    let mut state = load_provisioner_state(s3, bucket).await?;

    for resource in resources.iter().rev() {
        let resource_type = resource.resource_type();
        if let Some(rs) = state.resources.get(resource_type) {
            tracing::info!(resource_type = %resource_type, "destroying resource");
            resource.delete(&rs.resource_id).await?;
        }
    }

    state.resources.clear();
    save_provisioner_state(s3, bucket, &state).await?;

    Ok(())
}
