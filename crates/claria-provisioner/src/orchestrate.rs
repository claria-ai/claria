use std::collections::{HashMap, HashSet};

use crate::addr::ResourceAddr;
use crate::error::ProvisionerError;
use crate::manifest::{Lifecycle, Manifest, ResourceSpec};
use crate::persistence::StatePersistence;
use crate::plan::{Action, Cause, PlanEntry};
use crate::state::{ProvisionerState, ResourceState, ResourceStatus};
use crate::syncer::ResourceSyncer;

/// Scan all resources and produce an annotated plan.
///
/// The plan is a flat `Vec<PlanEntry>` — one entry per manifest spec, plus
/// orphan entries for resources in state but not in the current manifest.
pub async fn plan(
    syncers: &[Box<dyn ResourceSyncer>],
    state: &ProvisionerState,
) -> Result<Vec<PlanEntry>, ProvisionerError> {
    let manifest_upgraded = state
        .manifest_version
        .map_or(true, |v| v < Manifest::VERSION);
    let known_addrs: HashSet<_> = state.resources.keys().cloned().collect();
    let mut entries = Vec::new();

    // 1. Walk syncers in order — read + diff each resource
    for syncer in syncers {
        let spec = syncer.spec();
        let actual = syncer.read().await?;

        let entry = match (spec.lifecycle, &actual) {
            // Data source missing → precondition failure
            (Lifecycle::Data, None) => PlanEntry {
                spec: spec.clone(),
                action: Action::PreconditionFailed,
                cause: Cause::Drift,
                drift: vec![],
            },

            // Data source exists → check it matches
            (Lifecycle::Data, Some(actual)) => {
                let drift = syncer.diff(actual);
                PlanEntry {
                    spec: spec.clone(),
                    action: if drift.is_empty() {
                        Action::Ok
                    } else {
                        Action::PreconditionFailed
                    },
                    cause: if drift.is_empty() {
                        Cause::InSync
                    } else if manifest_upgraded {
                        Cause::ManifestChanged
                    } else {
                        Cause::Drift
                    },
                    drift,
                }
            }

            // Managed resource missing → needs creation
            (Lifecycle::Managed, None) => PlanEntry {
                spec: spec.clone(),
                action: Action::Create,
                cause: if !manifest_upgraded || known_addrs.contains(&spec.addr()) {
                    Cause::FirstProvision
                } else {
                    Cause::ManifestChanged
                },
                drift: vec![],
            },

            // Managed resource exists → check for drift
            (Lifecycle::Managed, Some(actual)) => {
                let drift = syncer.diff(actual);
                PlanEntry {
                    spec: spec.clone(),
                    action: if drift.is_empty() {
                        Action::Ok
                    } else {
                        Action::Modify
                    },
                    cause: if drift.is_empty() {
                        Cause::InSync
                    } else if manifest_upgraded {
                        Cause::ManifestChanged
                    } else {
                        Cause::Drift
                    },
                    drift,
                }
            }
        };
        entries.push(entry);
    }

    // 2. Orphan pass — resources in state but not in manifest
    let manifest_addrs: HashSet<_> = syncers.iter().map(|s| s.spec().addr()).collect();
    for addr in state.resources.keys() {
        if !manifest_addrs.contains(addr) {
            entries.push(PlanEntry {
                spec: ResourceSpec::orphaned(addr),
                action: Action::Delete,
                cause: Cause::Orphaned,
                drift: vec![],
            });
        }
    }

    Ok(entries)
}

/// Execute all actionable entries in the plan.
///
/// Creates in manifest order (dependencies satisfied by position),
/// modifies in manifest order, deletes in reverse order (dependents first).
pub async fn execute(
    entries: &[PlanEntry],
    syncers: &[Box<dyn ResourceSyncer>],
    state: &mut ProvisionerState,
    persistence: &StatePersistence,
) -> Result<(), ProvisionerError> {
    let syncer_map: HashMap<ResourceAddr, &dyn ResourceSyncer> = syncers
        .iter()
        .map(|s| (s.spec().addr(), s.as_ref()))
        .collect();

    // Creates — manifest order
    for entry in entries.iter().filter(|e| e.action == Action::Create) {
        let addr = entry.spec.addr();
        let syncer = syncer_map.get(&addr).ok_or_else(|| {
            ProvisionerError::ResourceNotFound {
                resource_type: addr.resource_type.clone(),
                resource_id: addr.resource_name.clone(),
            }
        })?;

        tracing::info!(addr = %addr, "creating resource");
        let result = syncer.create().await?;
        state.resources.insert(
            addr,
            ResourceState {
                resource_type: entry.spec.resource_type.clone(),
                resource_id: entry.spec.resource_name.clone(),
                status: ResourceStatus::Created,
                properties: result,
            },
        );
        persistence.flush(state).await?;
    }

    // Modifies — manifest order
    for entry in entries.iter().filter(|e| e.action == Action::Modify) {
        let addr = entry.spec.addr();
        let syncer = syncer_map.get(&addr).ok_or_else(|| {
            ProvisionerError::ResourceNotFound {
                resource_type: addr.resource_type.clone(),
                resource_id: addr.resource_name.clone(),
            }
        })?;

        tracing::info!(addr = %addr, "updating resource");
        let result = syncer.update().await?;
        if let Some(rs) = state.resources.get_mut(&addr) {
            rs.status = ResourceStatus::Updated;
            rs.properties = result;
        }
        persistence.flush(state).await?;
    }

    // Deletes — reverse order (dependents before dependencies)
    for entry in entries
        .iter()
        .filter(|e| e.action == Action::Delete)
        .rev()
    {
        let addr = entry.spec.addr();
        if let Some(syncer) = syncer_map.get(&addr) {
            tracing::info!(addr = %addr, "destroying resource");
            syncer.destroy().await?;
        }
        state.resources.remove(&addr);
        persistence.flush(state).await?;
    }

    // Stamp manifest version
    state.manifest_version = Some(Manifest::VERSION);
    persistence.flush(state).await?;

    Ok(())
}

/// Destroy all managed resources.
///
/// Marks every resource in state as an orphan and walks the delete list
/// in reverse syncer order.
pub async fn destroy_all(
    syncers: &[Box<dyn ResourceSyncer>],
    state: &mut ProvisionerState,
    persistence: &StatePersistence,
) -> Result<(), ProvisionerError> {
    // Walk syncers in reverse (dependents first)
    for syncer in syncers.iter().rev() {
        let addr = syncer.spec().addr();
        if state.resources.contains_key(&addr) {
            tracing::info!(addr = %addr, "destroying resource");
            syncer.destroy().await?;
            state.resources.remove(&addr);
            persistence.flush(state).await?;
        }
    }

    state.resources.clear();
    state.manifest_version = None;
    persistence.flush(state).await?;

    Ok(())
}
