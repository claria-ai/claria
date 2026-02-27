use crate::error::ProvisionerError;
use crate::plan::{ActionType, ExecutionPlan};
use crate::resource::Resource;
use crate::state::{ProvisionerState, ResourceState, ResourceStatus};

/// Execute an execution plan, applying all actions.
pub async fn execute_plan(
    plan: &ExecutionPlan,
    resources: &[Box<dyn Resource>],
    state: &mut ProvisionerState,
) -> Result<(), ProvisionerError> {
    for action in &plan.actions {
        if action.action == ActionType::NoOp {
            continue;
        }

        let resource = resources
            .iter()
            .find(|r| r.resource_type() == action.resource_type)
            .ok_or_else(|| ProvisionerError::ResourceNotFound {
                resource_type: action.resource_type.clone(),
                resource_id: action.resource_id.clone(),
            })?;

        match action.action {
            ActionType::Create => {
                tracing::info!(
                    resource_type = %action.resource_type,
                    "creating resource"
                );
                let result = resource.create().await?;
                state.resources.insert(
                    action.resource_type.clone(),
                    ResourceState {
                        resource_type: action.resource_type.clone(),
                        resource_id: result.resource_id,
                        status: ResourceStatus::Created,
                        properties: result.properties,
                    },
                );
            }
            ActionType::Update => {
                tracing::info!(
                    resource_type = %action.resource_type,
                    resource_id = %action.resource_id,
                    "updating resource"
                );
                let result = resource.update(&action.resource_id).await?;
                if let Some(rs) = state.resources.get_mut(&action.resource_type) {
                    rs.status = ResourceStatus::Updated;
                    rs.properties = result.properties;
                }
            }
            ActionType::Delete => {
                tracing::info!(
                    resource_type = %action.resource_type,
                    resource_id = %action.resource_id,
                    "deleting resource"
                );
                resource.delete(&action.resource_id).await?;
                if let Some(rs) = state.resources.get_mut(&action.resource_type) {
                    rs.status = ResourceStatus::Deleted;
                }
            }
            ActionType::NoOp => {}
        }
    }

    Ok(())
}
