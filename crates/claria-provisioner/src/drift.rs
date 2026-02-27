use crate::error::ProvisionerError;
use crate::plan::{ActionType, ExecutionPlan, PlanAction};
use crate::resource::Resource;
use crate::state::ProvisionerState;

/// Compare desired state against actual AWS state and produce an execution plan.
pub async fn detect_drift(
    state: &ProvisionerState,
    resources: &[Box<dyn Resource>],
) -> Result<ExecutionPlan, ProvisionerError> {
    let mut actions = Vec::new();

    for resource in resources {
        let resource_type = resource.resource_type().to_string();
        let current = resource.current_state().await;

        match (state.resources.get(&resource_type), current) {
            // Resource exists in state and in AWS — check for drift
            (Some(expected), Ok(Some(actual))) => {
                if expected.properties != actual {
                    actions.push(PlanAction {
                        resource_type,
                        resource_id: expected.resource_id.clone(),
                        action: ActionType::Update,
                        reason: "drift detected: actual state differs from expected".to_string(),
                    });
                } else {
                    actions.push(PlanAction {
                        resource_type,
                        resource_id: expected.resource_id.clone(),
                        action: ActionType::NoOp,
                        reason: "in sync".to_string(),
                    });
                }
            }
            // Resource exists in state but not in AWS — needs recreation
            (Some(expected), Ok(None)) => {
                actions.push(PlanAction {
                    resource_type,
                    resource_id: expected.resource_id.clone(),
                    action: ActionType::Create,
                    reason: "resource missing from AWS, needs recreation".to_string(),
                });
            }
            // Resource not in state — needs creation
            (None, _) => {
                actions.push(PlanAction {
                    resource_type,
                    resource_id: String::new(),
                    action: ActionType::Create,
                    reason: "resource not yet provisioned".to_string(),
                });
            }
            // Error querying AWS
            (Some(expected), Err(e)) => {
                tracing::warn!(
                    resource_type = %resource_type,
                    error = %e,
                    "failed to query current state"
                );
                actions.push(PlanAction {
                    resource_type,
                    resource_id: expected.resource_id.clone(),
                    action: ActionType::NoOp,
                    reason: format!("unable to determine state: {e}"),
                });
            }
        }
    }

    Ok(ExecutionPlan { actions })
}
