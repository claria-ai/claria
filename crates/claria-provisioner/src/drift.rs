use serde::{Deserialize, Serialize};
use specta::Type;

use crate::resource::Resource;
use crate::scan::{ScanResult, ScanStatus};
use crate::state::ProvisionerState;

// Old plan types kept inline for backward compat during migration.
// These will be deleted in Phase 6 along with this entire file.

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct OldPlanEntry {
    pub resource_type: String,
    pub resource_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct OldPlan {
    pub ok: Vec<OldPlanEntry>,
    pub modify: Vec<OldPlanEntry>,
    pub create: Vec<OldPlanEntry>,
    pub delete: Vec<OldPlanEntry>,
}

impl OldPlan {
    pub fn has_changes(&self) -> bool {
        !self.modify.is_empty() || !self.create.is_empty() || !self.delete.is_empty()
    }
}

/// Compare scan results against provisioner state and produce a four-bucket plan.
pub fn build_plan(
    state: &ProvisionerState,
    scan_results: &[ScanResult],
    resources: &[Box<dyn Resource>],
) -> OldPlan {
    let mut plan = OldPlan::default();

    for result in scan_results {
        // Look up state by resource_type — find any ResourceAddr with matching type
        let in_state = state
            .resources
            .iter()
            .find(|(addr, _)| addr.resource_type == result.resource_type)
            .map(|(_, rs)| rs);

        match (in_state, result.status) {
            (Some(expected), ScanStatus::Found) => {
                if let Some(actual) = &result.properties {
                    if expected.properties == *actual {
                        plan.ok.push(OldPlanEntry {
                            resource_type: result.resource_type.clone(),
                            resource_id: expected.resource_id.clone(),
                            reason: "in sync".to_string(),
                        });
                    } else {
                        plan.modify.push(OldPlanEntry {
                            resource_type: result.resource_type.clone(),
                            resource_id: expected.resource_id.clone(),
                            reason: "drift detected: actual state differs from expected"
                                .to_string(),
                        });
                    }
                } else {
                    plan.ok.push(OldPlanEntry {
                        resource_type: result.resource_type.clone(),
                        resource_id: expected.resource_id.clone(),
                        reason: "found but no properties to compare".to_string(),
                    });
                }
            }
            (None, ScanStatus::Found) => {
                let resource_id = result.resource_id.clone().unwrap_or_default();
                plan.ok.push(OldPlanEntry {
                    resource_type: result.resource_type.clone(),
                    resource_id,
                    reason: "exists in AWS, not yet tracked in state".to_string(),
                });
            }
            (Some(expected), ScanStatus::NotFound) => {
                plan.create.push(OldPlanEntry {
                    resource_type: result.resource_type.clone(),
                    resource_id: expected.resource_id.clone(),
                    reason: "resource missing from AWS, needs recreation".to_string(),
                });
            }
            (None, ScanStatus::NotFound) => {
                plan.create.push(OldPlanEntry {
                    resource_type: result.resource_type.clone(),
                    resource_id: String::new(),
                    reason: "resource not yet provisioned".to_string(),
                });
            }
            (_, ScanStatus::Error) => {
                let error_msg = result
                    .error
                    .clone()
                    .unwrap_or_else(|| "unknown error".to_string());
                plan.ok.push(OldPlanEntry {
                    resource_type: result.resource_type.clone(),
                    resource_id: in_state
                        .map(|s| s.resource_id.clone())
                        .unwrap_or_default(),
                    reason: format!("unable to determine state: {error_msg}"),
                });
            }
        }
    }

    // Check for resources in state that weren't scanned
    for (addr, rs) in &state.resources {
        let was_scanned = scan_results
            .iter()
            .any(|r| r.resource_type == addr.resource_type);
        let has_resource = resources
            .iter()
            .any(|r| r.resource_type() == addr.resource_type);

        if !was_scanned && !has_resource {
            plan.delete.push(OldPlanEntry {
                resource_type: addr.resource_type.clone(),
                resource_id: rs.resource_id.clone(),
                reason: "resource in state but no longer managed".to_string(),
            });
        }
    }

    plan
}
