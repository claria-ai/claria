use crate::addr::ResourceAddr;
use crate::drift::OldPlan;
use crate::error::ProvisionerError;
use crate::persistence::StatePersistence;
use crate::resource::Resource;
use crate::state::{ProvisionerState, ResourceState, ResourceStatus};

/// Execute a plan: process creates, then modifies, then deletes.
/// Flushes state to disk + S3 after each resource action.
pub async fn execute_plan(
    plan: &OldPlan,
    resources: &[Box<dyn Resource>],
    state: &mut ProvisionerState,
    persistence: &StatePersistence,
) -> Result<(), ProvisionerError> {
    // Process creates
    for entry in &plan.create {
        let resource = find_resource(resources, &entry.resource_type)?;

        tracing::info!(
            resource_type = %entry.resource_type,
            "creating resource"
        );
        let result = resource.create().await?;
        let addr = ResourceAddr {
            resource_type: entry.resource_type.clone(),
            resource_name: result.resource_id.clone(),
        };
        state.resources.insert(
            addr,
            ResourceState {
                resource_type: entry.resource_type.clone(),
                resource_id: result.resource_id,
                status: ResourceStatus::Created,
                properties: result.properties,
            },
        );
        persistence.flush(state).await?;
    }

    // Process modifies
    for entry in &plan.modify {
        let resource = find_resource(resources, &entry.resource_type)?;

        tracing::info!(
            resource_type = %entry.resource_type,
            resource_id = %entry.resource_id,
            "updating resource"
        );
        let result = resource.update(&entry.resource_id).await?;
        // Find the matching state entry by resource_type
        if let Some((_, rs)) = state
            .resources
            .iter_mut()
            .find(|(addr, _)| addr.resource_type == entry.resource_type)
        {
            rs.status = ResourceStatus::Updated;
            rs.properties = result.properties;
        }
        persistence.flush(state).await?;
    }

    // Process deletes (state cleanup only)
    for entry in &plan.delete {
        tracing::info!(
            resource_type = %entry.resource_type,
            resource_id = %entry.resource_id,
            "removing stale state entry"
        );
        state
            .resources
            .retain(|addr, _| addr.resource_type != entry.resource_type);
        persistence.flush(state).await?;
    }

    Ok(())
}

fn find_resource<'a>(
    resources: &'a [Box<dyn Resource>],
    resource_type: &str,
) -> Result<&'a dyn Resource, ProvisionerError> {
    resources
        .iter()
        .find(|r| r.resource_type() == resource_type)
        .map(|b| b.as_ref())
        .ok_or_else(|| ProvisionerError::ResourceNotFound {
            resource_type: resource_type.to_string(),
            resource_id: String::new(),
        })
}
