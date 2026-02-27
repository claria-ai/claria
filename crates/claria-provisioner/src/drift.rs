use crate::plan::{Plan, PlanEntry};
use crate::resource::Resource;
use crate::scan::{ScanResult, ScanStatus};
use crate::state::ProvisionerState;

/// Compare scan results against provisioner state and produce a four-bucket plan.
pub fn build_plan(
    state: &ProvisionerState,
    scan_results: &[ScanResult],
    resources: &[Box<dyn Resource>],
) -> Plan {
    let mut plan = Plan::default();

    for result in scan_results {
        let in_state = state.resources.get(&result.resource_type);

        match (in_state, result.status) {
            // Resource found in AWS and in state — check for drift
            (Some(expected), ScanStatus::Found) => {
                if let Some(actual) = &result.properties {
                    if expected.properties == *actual {
                        plan.ok.push(PlanEntry {
                            resource_type: result.resource_type.clone(),
                            resource_id: expected.resource_id.clone(),
                            reason: "in sync".to_string(),
                        });
                    } else {
                        plan.modify.push(PlanEntry {
                            resource_type: result.resource_type.clone(),
                            resource_id: expected.resource_id.clone(),
                            reason: "drift detected: actual state differs from expected"
                                .to_string(),
                        });
                    }
                } else {
                    plan.ok.push(PlanEntry {
                        resource_type: result.resource_type.clone(),
                        resource_id: expected.resource_id.clone(),
                        reason: "found but no properties to compare".to_string(),
                    });
                }
            }

            // Resource found in AWS but not in state — record it
            (None, ScanStatus::Found) => {
                let resource_id = result.resource_id.clone().unwrap_or_default();
                plan.ok.push(PlanEntry {
                    resource_type: result.resource_type.clone(),
                    resource_id,
                    reason: "exists in AWS, not yet tracked in state".to_string(),
                });
            }

            // Resource in state but not found in AWS — needs recreation
            (Some(expected), ScanStatus::NotFound) => {
                plan.create.push(PlanEntry {
                    resource_type: result.resource_type.clone(),
                    resource_id: expected.resource_id.clone(),
                    reason: "resource missing from AWS, needs recreation".to_string(),
                });
            }

            // Resource not in state and not found in AWS — needs creation
            (None, ScanStatus::NotFound) => {
                plan.create.push(PlanEntry {
                    resource_type: result.resource_type.clone(),
                    resource_id: String::new(),
                    reason: "resource not yet provisioned".to_string(),
                });
            }

            // Error scanning — don't take action on unknown state
            (_, ScanStatus::Error) => {
                let error_msg = result
                    .error
                    .clone()
                    .unwrap_or_else(|| "unknown error".to_string());
                plan.ok.push(PlanEntry {
                    resource_type: result.resource_type.clone(),
                    resource_id: in_state
                        .map(|s| s.resource_id.clone())
                        .unwrap_or_default(),
                    reason: format!("unable to determine state: {error_msg}"),
                });
            }
        }
    }

    // Check for resources in state that weren't scanned (stale entries)
    for (resource_type, rs) in &state.resources {
        let was_scanned = scan_results
            .iter()
            .any(|r| r.resource_type == *resource_type);
        let has_resource = resources
            .iter()
            .any(|r| r.resource_type() == resource_type);

        if !was_scanned && !has_resource {
            plan.delete.push(PlanEntry {
                resource_type: resource_type.clone(),
                resource_id: rs.resource_id.clone(),
                reason: "resource in state but no longer managed".to_string(),
            });
        }
    }

    plan
}
