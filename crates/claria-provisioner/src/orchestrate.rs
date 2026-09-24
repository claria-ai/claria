use std::collections::{HashMap, HashSet};

use crate::{
    addr::ResourceAddr,
    error::ProvisionerError,
    manifest::{Lifecycle, Manifest, ResourceSpec},
    persistence::StatePersistence,
    plan::{Action, Cause, PlanEntry},
    state::{ProvisionerState, ResourceState, ResourceStatus},
    syncer::{ResourceSyncer, compute_drift},
};

/// Build a single plan entry from a syncer and its read result.
///
/// Pure structural comparison: desired state vs actual state. No version
/// tracking — either the resource exists with the right properties, or it
/// needs reconciliation.
pub fn build_plan_entry(
    syncer: &dyn ResourceSyncer,
    actual: Option<serde_json::Value>,
) -> PlanEntry {
    let spec = syncer.spec();
    match (spec.lifecycle, &actual) {
        // Data source missing → precondition failure
        (Lifecycle::Data, None) => PlanEntry {
            spec: spec.clone(),
            action: Action::PreconditionFailed,
            cause: Cause::Missing,
            drift: vec![],
            actual: None,
            error: None,
        },

        // Data source exists → check it matches
        (Lifecycle::Data, Some(actual_val)) => {
            let drift = compute_drift(&syncer.desired_state(), &syncer.current_state(actual_val));
            PlanEntry {
                spec: spec.clone(),
                action: if drift.is_empty() {
                    Action::Ok
                } else {
                    Action::PreconditionFailed
                },
                cause: if drift.is_empty() {
                    Cause::InSync
                } else {
                    Cause::Drift
                },
                drift,
                actual: Some(actual_val.clone()),
                error: None,
            }
        }

        // Managed resource missing → needs creation
        (Lifecycle::Managed, None) => PlanEntry {
            spec: spec.clone(),
            action: Action::Create,
            cause: Cause::Missing,
            drift: vec![],
            actual: None,
            error: None,
        },

        // Managed resource exists → check for drift
        (Lifecycle::Managed, Some(actual_val)) => {
            let drift = compute_drift(&syncer.desired_state(), &syncer.current_state(actual_val));
            PlanEntry {
                spec: spec.clone(),
                action: if drift.is_empty() {
                    Action::Ok
                } else {
                    Action::Modify
                },
                cause: if drift.is_empty() {
                    Cause::InSync
                } else {
                    Cause::Drift
                },
                drift,
                actual: Some(actual_val.clone()),
                error: None,
            }
        }
    }
}

/// Build the entry for a resource whose read failed.
///
/// The one honest answer to "is it there?" when AWS refused to say. Kept
/// beside [`build_plan_entry`] so the two ways an entry can be made stay in
/// one file, and separate from it because there is nothing to compare: no
/// desired state, no drift, no `actual`.
///
/// Reported for `Data` and `Managed` alike. A precondition Claria cannot read
/// is not a precondition it may call satisfied.
pub fn build_unreadable_entry(syncer: &dyn ResourceSyncer, error: &ProvisionerError) -> PlanEntry {
    PlanEntry {
        spec: syncer.spec().clone(),
        action: Action::Unknown,
        cause: Cause::Unreadable,
        drift: vec![],
        actual: None,
        error: Some(error.to_string()),
    }
}

/// Find orphaned resources — those in state but not in the current manifest.
///
/// Compared against the whole manifest, never against a scope-filtered syncer
/// list. A pass that builds only the elevated syncers would otherwise call
/// every regular resource an orphan and queue the data bucket for deletion.
pub fn find_orphans(manifest: &Manifest, state: &ProvisionerState) -> Vec<PlanEntry> {
    let manifest_addrs: HashSet<_> = manifest.specs.iter().map(|s| s.addr()).collect();
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
            error: None,
        })
        .collect()
}

/// Record the result of a create or update against the resource's address.
///
/// Upserts. A `Modify` on a resource the state file had never heard of still
/// lands, which is what keeps teardown able to find it afterwards.
pub fn record_applied(
    state: &mut ProvisionerState,
    spec: &ResourceSpec,
    properties: serde_json::Value,
    status: ResourceStatus,
) {
    state.resources.insert(
        spec.addr(),
        ResourceState {
            resource_type: spec.resource_type.clone(),
            resource_id: spec.resource_name.clone(),
            status,
            properties,
        },
    );
}

/// Record every managed resource this scan actually found.
///
/// State answers one question — which resources Claria is responsible for — so
/// [`destroy_all`] and [`find_orphans`] have something to work from. Drift is
/// never read from here; [`build_plan_entry`] derives it from live reads. This
/// is what lets a deleted or reset state file rebuild itself from a scan,
/// instead of leaving already-conformant resources unowned forever.
///
/// Additive on purpose. A read Claria could not complete arrives here as an
/// [`Action::Unknown`] entry with no `actual`, and is skipped rather than
/// treated as an absence — dropping a record on that basis is how a bucket
/// survives a teardown that reported success. Stale rows cost nothing, because
/// nothing reads them to decide drift.
///
/// Only meaningful for a scan of the whole manifest — given a scope-filtered
/// scan, resources outside that scope simply go unrecorded.
///
/// Returns true when the record changed and the caller should flush.
pub fn reconcile_state(entries: &[PlanEntry], state: &mut ProvisionerState) -> bool {
    let mut changed = false;

    for entry in entries {
        // Data sources are preconditions Claria reads, not resources it owns.
        if entry.spec.lifecycle != Lifecycle::Managed || entry.cause == Cause::Orphaned {
            continue;
        }
        let Some(actual) = &entry.actual else {
            continue;
        };
        let addr = entry.spec.addr();
        if state.resources.contains_key(&addr) {
            continue;
        }

        tracing::info!(addr = %addr, "recording existing resource as managed");
        record_applied(state, &entry.spec, actual.clone(), ResourceStatus::Unknown);
        changed = true;
    }

    changed
}

/// Log a summary of the scan results.
///
/// Unreadable resources are counted apart from out-of-sync ones. They are not
/// drift — nobody knows whether they have drifted — and a reader who cannot
/// tell the two apart cannot tell a conformant account from an unseen one.
pub fn log_scan_summary(entries: &[PlanEntry]) {
    let total = entries.len();
    let conformant = entries.iter().filter(|e| e.action == Action::Ok).count();
    let unreadable: Vec<&str> = entries
        .iter()
        .filter(|e| e.action == Action::Unknown)
        .map(|e| e.spec.label.as_str())
        .collect();
    let out_of_sync: Vec<&str> = entries
        .iter()
        .filter(|e| e.action != Action::Ok && e.action != Action::Unknown)
        .map(|e| e.spec.label.as_str())
        .collect();

    if out_of_sync.is_empty() && unreadable.is_empty() {
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
            unreadable_count = unreadable.len(),
            unreadable = unreadable.join(", "),
            "scan complete"
        );
    }
}

/// Scan all resources and produce an annotated plan.
///
/// The plan is a flat `Vec<PlanEntry>` — one entry per syncer, plus orphan
/// entries when `orphan_basis` is supplied. Pass it only for a scan of the
/// whole manifest; a scope-filtered pass has no business deciding what is
/// orphaned.
///
/// A read that fails produces an [`Action::Unknown`] entry rather than ending
/// the scan. One resource the credentials cannot see used to cost the operator
/// the whole plan, which is why every syncer learned to call a refused read
/// "absent" — the worst answer of the three, because a bucket full of PHI then
/// reads as one that needs creating.
pub async fn plan(
    syncers: &[Box<dyn ResourceSyncer>],
    orphan_basis: Option<(&Manifest, &ProvisionerState)>,
) -> Result<Vec<PlanEntry>, ProvisionerError> {
    let mut entries = Vec::new();

    tracing::info!(count = syncers.len(), "starting scan");

    for syncer in syncers.iter() {
        match syncer.read().await {
            Ok(actual) => entries.push(build_plan_entry(syncer.as_ref(), actual)),
            Err(error) => {
                tracing::warn!(
                    addr = %syncer.spec().addr(),
                    error = %error,
                    "could not read resource during scan"
                );
                entries.push(build_unreadable_entry(syncer.as_ref(), &error));
            }
        }
    }

    if let Some((manifest, state)) = orphan_basis {
        entries.extend(find_orphans(manifest, state));
    }
    log_scan_summary(&entries);

    Ok(entries)
}

/// Execute all actionable entries in the plan.
///
/// Creates and modifies run in a single pass in manifest order so that
/// dependencies are satisfied (e.g. bucket policy applied before CloudTrail
/// trail creation). Deletes run in reverse order (dependents first).
///
/// `syncers` must cover every address the plan acts on, orphans included — see
/// [`crate::build_orphan_syncers`]. An orphan with no syncer cannot be
/// destroyed, so its state record is deliberately left in place rather than
/// dropped.
///
/// [`Action::Unknown`] entries are acted on by neither pass, and that is the
/// point: the scan could not read the resource, so there is no basis for
/// creating, changing, or destroying it. They are left in the plan for the
/// operator to see.
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
        let syncer = syncer_map
            .get(&addr)
            .ok_or_else(|| ProvisionerError::ResourceNotFound {
                resource_type: addr.resource_type.clone(),
                resource_id: addr.resource_name.clone(),
            })?;

        let (status, result) = if entry.action == Action::Create {
            tracing::info!(addr = %addr, "creating resource");
            let result = syncer
                .create()
                .await
                .map_err(|e| e.with_resource(&entry.spec.label, &entry.spec.resource_name))?;
            (ResourceStatus::Created, result)
        } else {
            tracing::info!(addr = %addr, "updating resource");
            let result = syncer
                .update()
                .await
                .map_err(|e| e.with_resource(&entry.spec.label, &entry.spec.resource_name))?;
            (ResourceStatus::Updated, result)
        };
        record_applied(state, &entry.spec, result, status);
        persistence.flush(state).await?;
    }

    // Deletes — reverse order (dependents before dependencies)
    for entry in entries.iter().filter(|e| e.action == Action::Delete).rev() {
        let addr = entry.spec.addr();
        let Some(syncer) = syncer_map.get(&addr) else {
            // Nothing here can tear the resource down, so forgetting it would
            // strand it in the account with no record that it is there.
            tracing::warn!(
                addr = %addr,
                "no syncer for orphaned resource — leaving its state record in place"
            );
            continue;
        };
        tracing::info!(addr = %addr, "destroying resource");
        syncer
            .destroy()
            .await
            .map_err(|e| e.with_resource(&entry.spec.label, &entry.spec.resource_name))?;
        state.resources.remove(&addr);
        persistence.flush(state).await?;
    }

    persistence.flush(state).await?;

    Ok(())
}

/// Destroy every managed resource.
///
/// Walks syncers in reverse (dependents first) and destroys a resource when a
/// live read finds it **or** state records it. The union matters in both
/// directions: a state file that was reset still tears the account down, and a
/// read that fails closed does not quietly leave a bucket of PHI standing.
///
/// `Lifecycle::Data` entries are skipped — they are preconditions Claria reads,
/// not resources it created, and their syncers refuse to destroy anything.
///
/// Pass orphan syncers alongside the manifest ones for a teardown that leaves
/// nothing behind.
pub async fn destroy_all(
    syncers: &[Box<dyn ResourceSyncer>],
    state: &mut ProvisionerState,
    persistence: &StatePersistence,
) -> Result<(), ProvisionerError> {
    for syncer in syncers.iter().rev() {
        let spec = syncer.spec();
        if spec.lifecycle == Lifecycle::Data {
            continue;
        }
        let addr = spec.addr();

        let recorded = state.resources.contains_key(&addr);
        let present = match syncer.read().await {
            Ok(actual) => actual.is_some(),
            Err(error) => {
                tracing::warn!(
                    addr = %addr,
                    error = %error,
                    "could not read resource during teardown — falling back to the state record"
                );
                false
            }
        };

        if !recorded && !present {
            continue;
        }

        tracing::info!(addr = %addr, recorded, present, "destroying resource");
        syncer
            .destroy()
            .await
            .map_err(|e| e.with_resource(&spec.label, &spec.resource_name))?;
        state.resources.remove(&addr);
        persistence.flush(state).await?;
    }

    state.resources.clear();
    persistence.flush(state).await?;

    Ok(())
}
