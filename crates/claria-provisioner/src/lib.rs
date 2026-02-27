//! claria-provisioner
//!
//! IaC engine for provisioning and hardening Claria's AWS infrastructure.
//! Library consumed by the Tauri desktop app.
//!
//! Public API:
//! - `scan()` — query current state of all resources
//! - `build_plan()` — compare scan results against state, produce four-bucket plan
//! - `execute()` — apply a plan, flushing state after each action
//! - `provision()` — convenience: scan → plan → execute
//! - `destroy()` — tear down all managed resources

pub mod drift;
pub mod error;
pub mod persistence;
pub mod plan;
pub mod resource;
pub mod resources;
pub mod scan;
pub mod state;
pub mod sync;

pub use crate::drift::build_plan;
pub use crate::error::ProvisionerError;
pub use crate::persistence::StatePersistence;
pub use crate::plan::{Plan, PlanEntry};
pub use crate::resource::Resource;
pub use crate::scan::{scan, ScanResult, ScanStatus};
pub use crate::state::ProvisionerState;

/// Full provisioning: scan → plan → execute.
pub async fn provision(
    persistence: &StatePersistence,
    resources: &[Box<dyn Resource>],
) -> Result<(), ProvisionerError> {
    let mut state = persistence.load().await?;
    let scan_results = scan::scan(resources).await;
    let plan = drift::build_plan(&state, &scan_results, resources);

    if plan.has_changes() {
        tracing::info!(
            creates = plan.create.len(),
            modifies = plan.modify.len(),
            deletes = plan.delete.len(),
            "executing provisioning plan"
        );
        sync::execute_plan(&plan, resources, &mut state, persistence).await?;
    } else {
        tracing::info!("all resources in sync, no changes needed");
    }

    Ok(())
}

/// Destroy all managed resources in reverse order.
pub async fn destroy(
    persistence: &StatePersistence,
    resources: &[Box<dyn Resource>],
) -> Result<(), ProvisionerError> {
    let mut state = persistence.load().await?;

    for resource in resources.iter().rev() {
        let resource_type = resource.resource_type();
        if let Some(rs) = state.resources.get(resource_type) {
            tracing::info!(resource_type = %resource_type, "destroying resource");
            resource.delete(&rs.resource_id).await?;
        }
    }

    state.resources.clear();
    persistence.flush(&state).await?;

    Ok(())
}
