use std::collections::{HashMap, HashSet};

use crate::addr::ResourceAddr;
use crate::error::ProvisionerError;
use crate::manifest::{Lifecycle, Manifest, ResourceSpec};
use crate::persistence::StatePersistence;
use crate::plan::{Action, Cause, PlanEntry};
use crate::state::{ProvisionerState, ResourceState, ResourceStatus};
use crate::syncer::{compute_drift, ResourceSyncer};

/// Build a single plan entry from a syncer and its read result.
///
/// This is the core logic for classifying a resource scan result into an
/// action + cause. Extracted so callers can drive the scan loop themselves
/// (e.g. to emit progress events between reads).
pub fn build_plan_entry(
    syncer: &dyn ResourceSyncer,
    actual: Option<serde_json::Value>,
    manifest_upgraded: bool,
    known_addrs: &HashSet<ResourceAddr>,
) -> PlanEntry {
    let spec = syncer.spec();
    match (spec.lifecycle, &actual) {
        // Data source missing → precondition failure
        (Lifecycle::Data, None) => PlanEntry {
            spec: spec.clone(),
            action: Action::PreconditionFailed,
            cause: Cause::Drift,
            drift: vec![],
            actual: None,
        },

        // Data source exists → check it matches
        (Lifecycle::Data, Some(actual_val)) => {
            let drift =
                compute_drift(&syncer.desired_state(), &syncer.current_state(actual_val));
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
                actual: Some(actual_val.clone()),
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
            actual: None,
        },

        // Managed resource exists → check for drift
        (Lifecycle::Managed, Some(actual_val)) => {
            let drift =
                compute_drift(&syncer.desired_state(), &syncer.current_state(actual_val));
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
                actual: Some(actual_val.clone()),
            }
        }
    }
}

/// Find orphaned resources — those in state but not in the current manifest.
pub fn find_orphans(
    syncers: &[Box<dyn ResourceSyncer>],
    state: &ProvisionerState,
) -> Vec<PlanEntry> {
    let manifest_addrs: HashSet<_> = syncers.iter().map(|s| s.spec().addr()).collect();
    state
        .resources
        .keys()
        .filter(|addr| !manifest_addrs.contains(addr))
        .map(|addr| PlanEntry {
            spec: ResourceSpec::orphaned(addr),
            action: Action::Delete,
            cause: Cause::Orphaned,
            drift: vec![],
            actual: None,
        })
        .collect()
}

/// Log a summary of the scan results.
pub fn log_scan_summary(entries: &[PlanEntry]) {
    let total = entries.len();
    let conformant = entries.iter().filter(|e| e.action == Action::Ok).count();
    let out_of_sync: Vec<&str> = entries
        .iter()
        .filter(|e| e.action != Action::Ok)
        .map(|e| e.spec.label.as_str())
        .collect();

    if out_of_sync.is_empty() {
        tracing::info!(
            count = total,
            conformant,
            "scan complete — all resources conformant"
        );
    } else {
        tracing::info!(
            count = total,
            conformant,
            out_of_sync_count = out_of_sync.len(),
            out_of_sync = out_of_sync.join(", "),
            "scan complete"
        );
    }
}

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
        .is_none_or(|v| v < Manifest::VERSION);
    let known_addrs: HashSet<_> = state.resources.keys().cloned().collect();
    let mut entries = Vec::new();

    tracing::info!(count = syncers.len(), "starting scan");

    for syncer in syncers.iter() {
        let actual = syncer.read().await?;
        entries.push(build_plan_entry(syncer.as_ref(), actual, manifest_upgraded, &known_addrs));
    }

    entries.extend(find_orphans(syncers, state));
    log_scan_summary(&entries);

    Ok(entries)
}

/// Execute all actionable entries in the plan.
///
/// Creates and modifies run in a single pass in manifest order so that
/// dependencies are satisfied (e.g. bucket policy applied before CloudTrail
/// trail creation). Deletes run in reverse order (dependents first).
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

    // Creates and modifies — single pass in manifest order
    for entry in entries
        .iter()
        .filter(|e| e.action == Action::Create || e.action == Action::Modify)
    {
        let addr = entry.spec.addr();
        let syncer = syncer_map.get(&addr).ok_or_else(|| {
            ProvisionerError::ResourceNotFound {
                resource_type: addr.resource_type.clone(),
                resource_id: addr.resource_name.clone(),
            }
        })?;

        if entry.action == Action::Create {
            tracing::info!(addr = %addr, "creating resource");
            let result = syncer.create().await.map_err(|e| {
                e.with_resource(&entry.spec.label, &entry.spec.resource_name)
            })?;
            state.resources.insert(
                addr,
                ResourceState {
                    resource_type: entry.spec.resource_type.clone(),
                    resource_id: entry.spec.resource_name.clone(),
                    status: ResourceStatus::Created,
                    properties: result,
                },
            );
        } else {
            tracing::info!(addr = %addr, "updating resource");
            let result = syncer.update().await.map_err(|e| {
                e.with_resource(&entry.spec.label, &entry.spec.resource_name)
            })?;
            if let Some(rs) = state.resources.get_mut(&addr) {
                rs.status = ResourceStatus::Updated;
                rs.properties = result;
            }
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
            syncer.destroy().await.map_err(|e| {
                e.with_resource(&entry.spec.label, &entry.spec.resource_name)
            })?;
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
