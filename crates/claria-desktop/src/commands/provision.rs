//! Credential assessment and provisioner scan/apply commands.

use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CommandContext, CommandError, parse_uuid, run};
use crate::state::DesktopState;
use claria_desktop::config::{self, ClariaConfig, CredentialSource};
use claria_provisioner::{
    Action, CredentialScope, PlanEntry,
    account_setup::{AccessKeyInfo, CredentialAssessment, CredentialClass},
};

// ---------------------------------------------------------------------------
// Credential input — what the frontend may reference credentials by
// ---------------------------------------------------------------------------

/// Credentials as the frontend supplies them to provisioning commands.
///
/// Mirrors [`CredentialSource`] for the user-typed variants and adds
/// `AssumedRole`, an opaque handle to temporary STS credentials held in
/// [`DesktopState`] — the secret access key and session token from an
/// `assume_role` call never cross the IPC boundary.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialInput {
    Inline {
        access_key_id: String,
        secret_access_key: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        session_token: Option<String>,
    },
    Profile {
        profile_name: String,
    },
    DefaultChain,
    AssumedRole {
        handle: String,
    },
}

impl CredentialInput {
    /// Resolve to a concrete [`CredentialSource`], dereferencing an
    /// assumed-role handle through state.
    async fn resolve(
        self,
        state: &State<'_, DesktopState>,
    ) -> Result<CredentialSource, CommandError> {
        match self {
            CredentialInput::Inline {
                access_key_id,
                secret_access_key,
                session_token,
            } => Ok(CredentialSource::Inline {
                access_key_id,
                secret_access_key,
                session_token,
            }),
            CredentialInput::Profile { profile_name } => {
                Ok(CredentialSource::Profile { profile_name })
            }
            CredentialInput::DefaultChain => Ok(CredentialSource::DefaultChain),
            CredentialInput::AssumedRole { handle } => {
                let handle = parse_uuid(&handle)?;
                state
                    .assumed_role_credentials
                    .lock()
                    .await
                    .get(&handle)
                    .cloned()
                    .ok_or_else(|| {
                        CommandError::Msg(
                            "The assumed-role session is no longer available. Assume the role again."
                                .to_string(),
                        )
                    })
            }
        }
    }
}

/// What `assume_role` returns to the frontend: everything about the assumed
/// session except its secrets, plus the opaque handle later provisioning
/// commands exchange for them.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AssumedRoleSession {
    /// Opaque reference to the credentials held in memory.
    pub handle: String,
    /// Temporary access key ID (not secret; shown for operator recognition).
    pub access_key_id: String,
    /// When the temporary credentials expire (ISO 8601).
    pub expiration: Option<String>,
    /// The ARN of the assumed role.
    pub assumed_role_arn: String,
    /// The account ID of the sub-account we assumed into.
    pub account_id: String,
}

// ---------------------------------------------------------------------------
// Provisioner progress — streamed to the frontend via Channel<T>
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ProvisionerProgress {
    ScanStarted {
        label: String,
        index: u32,
        total: u32,
    },
    ScanCompleted {
        label: String,
        index: u32,
        total: u32,
    },
    ApplyStarted {
        label: String,
        action: String,
        index: u32,
        total: u32,
    },
    ApplyCompleted {
        label: String,
        action: String,
        index: u32,
        total: u32,
    },
    EscalationStep {
        label: String,
        status: String,
    },
}

// ---------------------------------------------------------------------------
// Credential commands — thin wrappers that delegate to the provisioner
// ---------------------------------------------------------------------------

/// Assess the provided credentials: validates them via STS and classifies
/// them as root / IAM admin / scoped Claria / insufficient.
///
/// The desktop app uses the returned `CredentialAssessment` to decide
/// which UI flow to present (bootstrap vs. straight to provisioning).
#[tauri::command]
#[specta::specta]
pub async fn assess_credentials(
    state: State<'_, DesktopState>,
    region: String,
    credentials: CredentialInput,
) -> Result<CredentialAssessment, String> {
    run("assess_credentials", async {
        let credentials = credentials.resolve(&state).await?;
        let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;
        Ok(claria_provisioner::assess_credentials(&sdk_config).await?)
    })
    .await
}

/// Assume a role in an AWS sub-account using parent-account credentials.
///
/// The operator provides their parent-account credentials and the sub-account
/// details. We call STS AssumeRole, hold the temporary credentials in memory,
/// and return an [`AssumedRoleSession`] whose handle later provisioning
/// commands exchange for them. Neither the secret access key nor the session
/// token ever reaches the frontend, and nothing is persisted to disk.
#[tauri::command]
#[specta::specta]
pub async fn assume_role(
    state: State<'_, DesktopState>,
    region: String,
    credentials: CredentialInput,
    account_id: String,
    role_name: String,
) -> Result<AssumedRoleSession, String> {
    run("assume_role", async {
        let credentials = credentials.resolve(&state).await?;
        let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;

        let role_arn = claria_provisioner::build_role_arn(&account_id, &role_name);

        let result = claria_provisioner::assume_role(&sdk_config, &role_arn, None).await?;

        let handle = uuid::Uuid::new_v4();
        {
            let mut sessions = state.assumed_role_credentials.lock().await;
            // One live assumed-role session at a time: stale sessions have
            // expired or been superseded, and holding them serves nothing.
            sessions.clear();
            sessions.insert(
                handle,
                CredentialSource::Inline {
                    access_key_id: result.access_key_id.clone(),
                    secret_access_key: result.secret_access_key,
                    session_token: Some(result.session_token),
                },
            );
        }

        Ok(AssumedRoleSession {
            handle: handle.to_string(),
            access_key_id: result.access_key_id,
            expiration: result.expiration,
            assumed_role_arn: result.assumed_role_arn,
            account_id: result.account_id,
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn list_aws_profiles() -> Result<Vec<String>, String> {
    Ok(claria_desktop::aws::list_aws_profiles())
}

// ---------------------------------------------------------------------------
// Access key management — for resolving the 2-key limit during bootstrap
// ---------------------------------------------------------------------------

/// List all access keys for the `claria-admin` IAM user, enriched with
/// last-used metadata.
///
/// Called when bootstrap fails due to the 2-key limit so the operator can
/// pick which key to delete.
#[tauri::command]
#[specta::specta]
pub async fn list_user_access_keys(
    state: State<'_, DesktopState>,
    region: String,
    credentials: CredentialInput,
) -> Result<Vec<AccessKeyInfo>, String> {
    run("list_user_access_keys", async {
        let credentials = credentials.resolve(&state).await?;
        let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;
        Ok(claria_provisioner::list_user_access_keys(&sdk_config).await?)
    })
    .await
}

/// Delete one access key belonging to the `claria-admin` IAM user.
///
/// Called after the operator picks a key to remove to make room for a
/// fresh one during bootstrap.
#[tauri::command]
#[specta::specta]
pub async fn delete_user_access_key(
    state: State<'_, DesktopState>,
    region: String,
    credentials: CredentialInput,
    access_key_id: String,
) -> Result<(), String> {
    run("delete_user_access_key", async {
        let credentials = credentials.resolve(&state).await?;
        let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;
        Ok(claria_provisioner::delete_user_access_key(&sdk_config, &access_key_id).await?)
    })
    .await
}

// ---------------------------------------------------------------------------
// IAM policy escalation — update policy with elevated credentials
// ---------------------------------------------------------------------------

/// Update the `ClariaProvisionerAccess` IAM policy using temporary elevated
/// credentials (root or admin).
///
/// The dashboard calls this when the manifest changes and requires IAM actions
/// not in the current policy. The elevated credentials are used once and
/// discarded — they are never persisted to disk.
#[tauri::command]
#[specta::specta]
pub async fn escalate_iam_policy(
    state: State<'_, DesktopState>,
    access_key_id: String,
    secret_access_key: String,
    on_progress: tauri::ipc::Channel<ProvisionerProgress>,
) -> Result<(), String> {
    run("escalate_iam_policy", async {
        let ctx = CommandContext::new(&state).await?;
        let cfg = ctx.cfg;

        let _ = on_progress.send(ProvisionerProgress::EscalationStep {
            label: "Building elevated permission client".into(),
            status: "in_progress".into(),
        });

        let elevated_config = claria_desktop::aws::build_aws_config(
            &cfg.region,
            &CredentialSource::Inline {
                access_key_id,
                secret_access_key,
                session_token: None,
            },
        )
        .await;

        let _ = on_progress.send(ProvisionerProgress::EscalationStep {
            label: "Building elevated permission client".into(),
            status: "done".into(),
        });
        let _ = on_progress.send(ProvisionerProgress::EscalationStep {
            label: "Updating IAM policy document".into(),
            status: "in_progress".into(),
        });

        claria_provisioner::update_iam_policy(&elevated_config, &cfg.system_name, &cfg.account_id)
            .await?;

        let _ = on_progress.send(ProvisionerProgress::EscalationStep {
            label: "Updating IAM policy document".into(),
            status: "done".into(),
        });

        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Provisioner commands — scan, plan, provision, destroy
// ---------------------------------------------------------------------------

/// Bring the ownership record in line with what a whole-manifest scan found.
///
/// The record answers one question — which resources is Claria responsible for —
/// and a scan is the only thing that can rebuild it. Without this, a resource
/// that was already conformant the first time Claria saw it is never recorded,
/// so a reset state file stays empty and teardown walks past everything in it.
///
/// Flushes only when the record changed, so a steady-state scan stays read-only.
async fn reconcile_and_flush(
    entries: &[PlanEntry],
    prov_state: &mut claria_provisioner::ProvisionerState,
    persistence: &claria_provisioner::StatePersistence,
) -> Result<(), CommandError> {
    if claria_provisioner::reconcile_state(entries, prov_state) {
        persistence.flush(prov_state).await?;
    }
    Ok(())
}

/// Helper: scan all resources concurrently (up to 5 at a time), streaming
/// progress events via the channel. Returns plan entries in manifest order.
///
/// `orphan_basis` supplies the manifest and state to derive orphan entries from,
/// and belongs only to a scan of the whole manifest. A scope-filtered pass must
/// pass `None`: comparing state against a partial syncer list calls every
/// resource outside that scope an orphan and queues it for deletion.
async fn scan_with_progress(
    syncers: &[Box<dyn claria_provisioner::ResourceSyncer>],
    orphan_basis: Option<(
        &claria_provisioner::Manifest,
        &claria_provisioner::ProvisionerState,
    )>,
    on_progress: &tauri::ipc::Channel<ProvisionerProgress>,
) -> Result<Vec<PlanEntry>, CommandError> {
    let total = syncers.len() as u32;

    tracing::info!(count = total, "starting scan");

    // Bounded-concurrent reads (up to 5 at a time) that come back in
    // manifest order, so the plan entries stay deterministic. The futures
    // are collected up front so the stream borrows them with a concrete
    // lifetime — mapping references straight into `buffered` trips a
    // higher-ranked-lifetime error.
    let scans: Vec<_> = syncers
        .iter()
        .enumerate()
        .map(|(i, syncer)| async move {
            let label = syncer.spec().label.clone();
            let _ = on_progress.send(ProvisionerProgress::ScanStarted {
                label: label.clone(),
                index: i as u32,
                total,
            });
            let actual = syncer.read().await;
            let _ = on_progress.send(ProvisionerProgress::ScanCompleted {
                label,
                index: i as u32,
                total,
            });
            (syncer, actual)
        })
        .collect();
    let results: Vec<_> = futures::stream::iter(scans).buffered(5).collect().await;

    let mut entries = Vec::with_capacity(results.len());
    for (syncer, actual_result) in results {
        let actual = actual_result?;
        entries.push(claria_provisioner::build_plan_entry(
            syncer.as_ref(),
            actual,
        ));
    }

    if let Some((manifest, prov_state)) = orphan_basis {
        entries.extend(claria_provisioner::find_orphans(manifest, prov_state));
    }
    claria_provisioner::log_scan_summary(&entries);

    Ok(entries)
}

/// Helper: execute all actionable entries with progress events.
async fn execute_with_progress(
    entries: &[PlanEntry],
    syncers: &[Box<dyn claria_provisioner::ResourceSyncer>],
    prov_state: &mut claria_provisioner::ProvisionerState,
    persistence: &claria_provisioner::StatePersistence,
    on_progress: &tauri::ipc::Channel<ProvisionerProgress>,
) -> Result<(), CommandError> {
    let actionable: Vec<_> = entries
        .iter()
        .filter(|e| e.action == Action::Create || e.action == Action::Modify)
        .collect();
    let action_total = actionable.len() as u32;

    let syncer_map: std::collections::HashMap<_, _> = syncers
        .iter()
        .map(|s| (s.spec().addr(), s.as_ref()))
        .collect();

    for (step_idx, entry) in actionable.iter().enumerate() {
        let addr = entry.spec.addr();
        let syncer = syncer_map.get(&addr).ok_or_else(|| {
            CommandError::Msg(format!(
                "no syncer for {} {}",
                addr.resource_type, addr.resource_name
            ))
        })?;

        let action_str = if entry.action == Action::Create {
            "create"
        } else {
            "modify"
        };
        let _ = on_progress.send(ProvisionerProgress::ApplyStarted {
            label: entry.spec.label.clone(),
            action: action_str.into(),
            index: step_idx as u32,
            total: action_total,
        });

        let (status, result) = if entry.action == Action::Create {
            tracing::info!(addr = %addr, "creating resource");
            let result = syncer
                .create()
                .await
                .map_err(|e| e.with_resource(&entry.spec.label, &entry.spec.resource_name))?;
            (claria_provisioner::state::ResourceStatus::Created, result)
        } else {
            tracing::info!(addr = %addr, "updating resource");
            let result = syncer
                .update()
                .await
                .map_err(|e| e.with_resource(&entry.spec.label, &entry.spec.resource_name))?;
            (claria_provisioner::state::ResourceStatus::Updated, result)
        };
        claria_provisioner::record_applied(prov_state, &entry.spec, result, status);
        persistence.flush(prov_state).await?;

        let _ = on_progress.send(ProvisionerProgress::ApplyCompleted {
            label: entry.spec.label.clone(),
            action: action_str.into(),
            index: step_idx as u32,
            total: action_total,
        });
    }

    // Deletes — reverse order.
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
        prov_state.resources.remove(&addr);
        persistence.flush(prov_state).await?;
    }

    Ok(())
}

/// Scan all resources and return an annotated plan.
///
/// Streams `ProvisionerProgress` events via the channel as each resource
/// is scanned. Scans up to 5 resources concurrently for speed.
#[tauri::command]
#[specta::specta]
pub async fn plan(
    state: State<'_, DesktopState>,
    on_progress: tauri::ipc::Channel<ProvisionerProgress>,
) -> Result<Vec<PlanEntry>, String> {
    run("plan", async {
        let ctx = CommandContext::new(&state).await?;
        let cfg = &ctx.cfg;
        // Probe Cost Explorer only for an operator already using it: the one
        // action it grants is billed per request, and this page scans on mount.
        let manifest =
            claria_provisioner::build_manifest(&cfg.account_id, &cfg.system_name, &cfg.region)
                .with_cost_explorer_probe(cfg.cost_explorer_enabled);
        let syncers = claria_provisioner::build_syncers(&ctx.sdk_config, &manifest, None);
        let persistence = claria_provisioner::build_persistence(
            &ctx.sdk_config,
            &cfg.system_name,
            &cfg.account_id,
            &config::provisioner_state_dir(&cfg.system_name)?,
        )?;
        let mut prov_state = persistence.load().await?;

        let entries =
            scan_with_progress(&syncers, Some((&manifest, &prov_state)), &on_progress).await?;

        reconcile_and_flush(&entries, &mut prov_state, &persistence).await?;

        Ok(entries)
    })
    .await
}

/// Execute all actionable entries in the plan.
///
/// Returns the updated plan (all entries should now be Ok).
/// Streams `ProvisionerProgress` events via the channel as each resource
/// is created/modified/deleted.
#[tauri::command]
#[specta::specta]
pub async fn apply(
    state: State<'_, DesktopState>,
    on_progress: tauri::ipc::Channel<ProvisionerProgress>,
) -> Result<Vec<PlanEntry>, String> {
    run("apply", async {
        let ctx = CommandContext::new(&state).await?;
        let cfg = &ctx.cfg;
        // Probe Cost Explorer only for an operator already using it: the one
        // action it grants is billed per request, and this page scans on mount.
        let manifest =
            claria_provisioner::build_manifest(&cfg.account_id, &cfg.system_name, &cfg.region)
                .with_cost_explorer_probe(cfg.cost_explorer_enabled);
        let syncers = claria_provisioner::build_syncers(&ctx.sdk_config, &manifest, None);
        let persistence = claria_provisioner::build_persistence(
            &ctx.sdk_config,
            &cfg.system_name,
            &cfg.account_id,
            &config::provisioner_state_dir(&cfg.system_name)?,
        )?;

        let mut prov_state = persistence.load().await?;

        // Scan first (with progress).
        let entries =
            scan_with_progress(&syncers, Some((&manifest, &prov_state)), &on_progress).await?;

        // Orphans are absent from the manifest, so their syncers have to be
        // rebuilt from state — without them the delete pass has nothing to call
        // and the only thing that happens is that Claria forgets them.
        let mut executable = claria_provisioner::build_syncers(&ctx.sdk_config, &manifest, None);
        executable.extend(claria_provisioner::build_orphan_syncers(
            &ctx.sdk_config,
            &manifest,
            &prov_state,
        ));

        // Execute (with progress).
        execute_with_progress(
            &entries,
            &executable,
            &mut prov_state,
            &persistence,
            &on_progress,
        )
        .await?;

        // Re-scan to show updated state (with progress).
        let entries =
            scan_with_progress(&syncers, Some((&manifest, &prov_state)), &on_progress).await?;

        reconcile_and_flush(&entries, &mut prov_state, &persistence).await?;

        Ok(entries)
    })
    .await
}

/// Destroy all managed resources with temporary elevated credentials.
///
/// The saved scoped credentials deliberately cannot erase S3 version history.
/// The supplied root or IAM administrator credentials are validated against
/// the configured account, used only for this teardown, and never persisted.
#[tauri::command]
#[specta::specta]
pub async fn destroy(
    state: State<'_, DesktopState>,
    elevated_credentials: CredentialInput,
) -> Result<(), String> {
    run("destroy", async {
        let elevated_credentials = elevated_credentials.resolve(&state).await?;
        let cfg = config::load_config()?;
        let elevated_config =
            claria_desktop::aws::build_aws_config(&cfg.region, &elevated_credentials).await;
        let assessment = claria_provisioner::assess_credentials(&elevated_config).await?;
        claria_provisioner::validate_teardown_credentials(&assessment, &cfg.account_id)?;

        let manifest =
            claria_provisioner::build_manifest(&cfg.account_id, &cfg.system_name, &cfg.region);
        let mut syncers = claria_provisioner::build_syncers(&elevated_config, &manifest, None);
        let persistence = claria_provisioner::build_persistence(
            &elevated_config,
            &cfg.system_name,
            &cfg.account_id,
            &config::provisioner_state_dir(&cfg.system_name)?,
        )?;

        let mut prov_state = persistence.load().await?;

        // Resources the manifest no longer declares are still the operator's to
        // be rid of; without these, teardown leaves them running.
        syncers.extend(claria_provisioner::build_orphan_syncers(
            &elevated_config,
            &manifest,
            &prov_state,
        ));

        claria_provisioner::destroy_all(&syncers, &mut prov_state, &persistence).await?;
        Ok(())
    })
    .await
}

/// Delete the provisioner state file (local + S3) so the next scan starts fresh.
///
/// Use this when state is incompatible with the current version of Claria.
/// AWS resources are not affected — the next scan will re-discover them.
#[tauri::command]
#[specta::specta]
pub async fn reset_provisioner_state(state: State<'_, DesktopState>) -> Result<(), String> {
    run("reset_provisioner_state", async {
        let ctx = CommandContext::new(&state).await?;
        let persistence = claria_provisioner::build_persistence(
            &ctx.sdk_config,
            &ctx.cfg.system_name,
            &ctx.cfg.account_id,
            &config::provisioner_state_dir(&ctx.cfg.system_name)?,
        )?;
        Ok(persistence.delete().await?)
    })
    .await
}

// ---------------------------------------------------------------------------
// Unified provision — single reconciliation flow with lazy escalation
// ---------------------------------------------------------------------------

/// Result of a provision scan. Contains everything the frontend needs to
/// render the plan and decide whether escalation is required.
#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct ProvisionScanResult {
    /// The full plan across all resources.
    pub entries: Vec<PlanEntry>,
    /// True if any `Elevated`-scope resource needs Create or Modify,
    /// meaning the user must provide admin/root credentials.
    pub needs_escalation: bool,
    /// The account ID (resolved via STS from the provided credentials).
    pub account_id: String,
}

/// Why the credential handoff could not mint a key for this computer.
///
/// IAM caps a user at two access keys, so onboarding a third machine against
/// an already-provisioned account is a routine outcome, not a crash. The
/// frontend uses this to offer key deletion instead of dead-ending.
#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct AccessKeyLimitReached {
    /// The IAM user whose key slots are full.
    pub user_name: String,
    /// How many keys AWS allows.
    pub limit: u32,
    /// The full error text, so the operator sees exactly what AWS said.
    pub message: String,
}

/// What `provision_apply` did.
///
/// Reconciliation normally ends with a fresh plan. The one recoverable
/// interruption is the IAM access-key ceiling during the first-run credential
/// handoff, which is reported here rather than as an opaque error string.
#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct ProvisionApplyOutcome {
    /// The post-apply plan. Empty when `access_key_limit` is set.
    pub entries: Vec<PlanEntry>,
    /// Set when the handoff stopped at the IAM two-key ceiling.
    pub access_key_limit: Option<AccessKeyLimitReached>,
}

/// Scan all resources using the provided credentials.
///
/// This is the entry point for both first-run and day-2 flows. On first run
/// the frontend passes the user's initial credentials; on day-2 the frontend
/// passes the saved scoped credentials (or calls `plan()` instead).
///
/// Returns a `ProvisionScanResult` that tells the frontend whether elevated
/// credentials are needed before applying.
#[tauri::command]
#[specta::specta]
pub async fn provision_scan(
    state: State<'_, DesktopState>,
    region: String,
    system_name: String,
    credentials: CredentialInput,
    on_progress: tauri::ipc::Channel<ProvisionerProgress>,
) -> Result<ProvisionScanResult, String> {
    run("provision_scan", async {
        let credentials = credentials.resolve(&state).await?;
        let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;

        // Resolve account ID via STS.
        let identity = claria_provisioner::account_setup::get_caller_identity(&sdk_config).await?;

        let manifest =
            claria_provisioner::build_manifest(&identity.account_id, &system_name, &region);
        let syncers = claria_provisioner::build_syncers(&sdk_config, &manifest, None);

        // Try to load state; fall back to empty state if persistence isn't set up yet.
        let state_dir = config::provisioner_state_dir(&system_name)?;
        let prov_state = match claria_provisioner::build_persistence(
            &sdk_config,
            &system_name,
            &identity.account_id,
            &state_dir,
        ) {
            Ok(p) => p.load().await.unwrap_or_else(|_| {
                claria_provisioner::ProvisionerState::new(
                    region.clone(),
                    claria_core::s3_keys::bucket_name(&identity.account_id, &system_name),
                )
            }),
            Err(_) => claria_provisioner::ProvisionerState {
                resources: Default::default(),
                region: region.clone(),
                bucket: claria_core::s3_keys::bucket_name(&identity.account_id, &system_name),
            },
        };

        let entries =
            scan_with_progress(&syncers, Some((&manifest, &prov_state)), &on_progress).await?;

        let needs_escalation = entries.iter().any(|e| {
            e.spec.credential_scope == CredentialScope::Elevated
                && (e.action == Action::Create || e.action == Action::Modify)
        });

        Ok(ProvisionScanResult {
            entries,
            needs_escalation,
            account_id: identity.account_id,
        })
    })
    .await
}

/// Apply all changes in one unified reconciliation.
///
/// Two-phase execution:
/// 1. If elevated resources need changes and `elevated_credentials` is provided,
///    execute elevated resources first (IAM user, policy).
/// 2. After elevated execution, create an access key for the claria-admin user
///    if no config exists yet (credential handoff from admin → scoped creds).
/// 3. Execute regular resources with the scoped credentials.
/// 4. Re-scan and return the updated plan.
///
/// Step 2 can hit IAM's two-access-key ceiling when the account has already
/// onboarded two computers. That is reported as
/// [`ProvisionApplyOutcome::access_key_limit`] so the caller can offer key
/// deletion and retry.
#[tauri::command]
#[specta::specta]
pub async fn provision_apply(
    state: State<'_, DesktopState>,
    region: String,
    system_name: String,
    credentials: CredentialInput,
    elevated_credentials: Option<CredentialInput>,
    on_progress: tauri::ipc::Channel<ProvisionerProgress>,
) -> Result<ProvisionApplyOutcome, String> {
    run("provision_apply", async {
        let credentials = credentials.resolve(&state).await?;
        let elevated_credentials = match elevated_credentials {
            Some(input) => Some(input.resolve(&state).await?),
            None => None,
        };
        let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;

        let identity = claria_provisioner::account_setup::get_caller_identity(&sdk_config).await?;

        let manifest =
            claria_provisioner::build_manifest(&identity.account_id, &system_name, &region);

        // We need persistence that can work even before the S3 bucket exists.
        // For local-only state during bootstrap, build persistence with the
        // elevated config (which can at least do local writes).
        let persistence = claria_provisioner::build_persistence(
            &sdk_config,
            &system_name,
            &identity.account_id,
            &config::provisioner_state_dir(&system_name)?,
        )?;

        let mut prov_state =
            persistence
                .load()
                .await
                .unwrap_or_else(|_| claria_provisioner::ProvisionerState {
                    resources: Default::default(),
                    region: region.clone(),
                    bucket: claria_core::s3_keys::bucket_name(&identity.account_id, &system_name),
                });

        // ── Phase 1: Elevated resources ──────────────────────────────────────
        // If we have elevated credentials, build elevated syncers and execute.

        if let Some(ref elevated_creds) = elevated_credentials {
            let elevated_config =
                claria_desktop::aws::build_aws_config(&region, elevated_creds).await;

            let elevated_syncers = claria_provisioner::build_syncers(
                &elevated_config,
                &manifest,
                Some(CredentialScope::Elevated),
            );

            // Scan elevated resources. No orphan basis: this pass only built
            // the elevated syncers, and every regular resource in state would
            // otherwise be called an orphan and queued for deletion.
            let elevated_entries =
                scan_with_progress(&elevated_syncers, None, &on_progress).await?;

            let has_elevated_work = elevated_entries
                .iter()
                .any(|e| e.action == Action::Create || e.action == Action::Modify);

            if has_elevated_work {
                execute_with_progress(
                    &elevated_entries,
                    &elevated_syncers,
                    &mut prov_state,
                    &persistence,
                    &on_progress,
                )
                .await?;
            }
        }

        // ── Credential handoff ───────────────────────────────────────────────
        // If no config exists yet, the IAM user was just created. Create an
        // access key so we can switch to scoped credentials.

        // Whether this run performs the credential handoff. Read once: the
        // branch below writes the config, so asking again afterwards answers
        // about the config this very call created.
        let handing_off = !config::has_config();

        let regular_config = if handing_off {
            // We need elevated creds for CreateAccessKey.
            let elevated_creds = elevated_credentials.as_ref().ok_or_else(|| {
                CommandError::Msg(
                    "Elevated credentials required to create access key for new IAM user"
                        .to_string(),
                )
            })?;
            let elevated_config =
                claria_desktop::aws::build_aws_config(&region, elevated_creds).await;

            let _ = on_progress.send(ProvisionerProgress::EscalationStep {
                label: "Creating access key for claria-admin".into(),
                status: "in_progress".into(),
            });

            let (key_id, secret) = match claria_provisioner::create_access_key(&elevated_config)
                .await
            {
                Ok(pair) => pair,
                // Recoverable: the operator can free a slot by deleting a key
                // belonging to a computer they no longer use.
                Err(claria_provisioner::ProvisionerError::AccessKeyLimitExceeded {
                    user_name,
                    limit,
                }) => {
                    let message = claria_provisioner::ProvisionerError::AccessKeyLimitExceeded {
                        user_name: user_name.clone(),
                        limit,
                    }
                    .to_string();
                    tracing::warn!(
                        user_name = %user_name,
                        limit,
                        "credential handoff blocked by the IAM access-key limit"
                    );
                    let _ = on_progress.send(ProvisionerProgress::EscalationStep {
                        label: "Creating access key for claria-admin".into(),
                        status: "failed".into(),
                    });
                    return Ok(ProvisionApplyOutcome {
                        entries: Vec::new(),
                        access_key_limit: Some(AccessKeyLimitReached {
                            user_name,
                            limit,
                            message,
                        }),
                    });
                }
                Err(e) => return Err(e.into()),
            };

            let _ = on_progress.send(ProvisionerProgress::EscalationStep {
                label: "Creating access key for claria-admin".into(),
                status: "done".into(),
            });

            // Validate new credentials (IAM is eventually consistent).
            let _ = on_progress.send(ProvisionerProgress::EscalationStep {
                label: "Validating new credentials".into(),
                status: "in_progress".into(),
            });

            claria_provisioner::validate_new_credentials(&key_id, &secret, &elevated_config)
                .await?;

            let _ = on_progress.send(ProvisionerProgress::EscalationStep {
                label: "Validating new credentials".into(),
                status: "done".into(),
            });

            // Save config with new scoped credentials.
            let cfg = ClariaConfig {
                config_version: 0,
                region: region.clone(),
                system_name: system_name.clone(),
                account_id: identity.account_id.clone(),
                created_at: jiff::Timestamp::now(),
                credentials: CredentialSource::Inline {
                    access_key_id: key_id.clone(),
                    secret_access_key: secret.clone(),
                    session_token: None,
                },
                preferred_model_id: None,
                cost_explorer_enabled: false,
                hourly_cost_data: false,
                prompt_caching_enabled: true,
                transcription: Default::default(),
                report_authoring: Default::default(),
                model_tuning: Default::default(),
                chat_streaming: Default::default(),
                draft_pipeline: Default::default(),
                security: Default::default(),
            };

            config::save_config(&cfg)?;

            let mut guard = state.config.lock().await;
            *guard = Some(cfg);
            drop(guard);

            // Build SDK config from new scoped credentials.
            claria_desktop::aws::build_aws_config(
                &region,
                &CredentialSource::Inline {
                    access_key_id: key_id,
                    secret_access_key: secret,
                    session_token: None,
                },
            )
            .await
        } else {
            // Config already exists — use the credentials that were passed in
            // (which should be the saved scoped credentials).
            sdk_config
        };

        // ── Phase 2: Regular resources ───────────────────────────────────────

        let regular_syncers = claria_provisioner::build_syncers(
            &regular_config,
            &manifest,
            Some(CredentialScope::Regular),
        );

        // Scope-filtered again, so no orphan basis — see phase 1.
        let regular_entries = scan_with_progress(&regular_syncers, None, &on_progress).await?;

        let has_regular_work = regular_entries.iter().any(|e| {
            e.action == Action::Create || e.action == Action::Modify || e.action == Action::Delete
        });

        if has_regular_work {
            // Rebuild persistence with regular config (S3 bucket should exist now
            // or be about to be created).
            let regular_persistence = claria_provisioner::build_persistence(
                &regular_config,
                &system_name,
                &identity.account_id,
                &config::provisioner_state_dir(&system_name)?,
            )?;

            execute_with_progress(
                &regular_entries,
                &regular_syncers,
                &mut prov_state,
                &regular_persistence,
                &on_progress,
            )
            .await?;
        }

        // ── Final re-scan ────────────────────────────────────────────────────
        // Build all syncers with regular config for the final scan.

        let all_syncers = claria_provisioner::build_syncers(&regular_config, &manifest, None);

        let entries =
            scan_with_progress(&all_syncers, Some((&manifest, &prov_state)), &on_progress).await?;

        reconcile_and_flush(&entries, &mut prov_state, &persistence).await?;

        // Last, so the scoped credentials are minted, validated, saved and
        // proven by a full pass before the credential they replace is
        // destroyed — and so the bucket exists to record it in.
        if handing_off && let Some(ref elevated_creds) = elevated_credentials {
            retire_root_access_key(&state, &region, elevated_creds, &on_progress).await;
        }

        Ok(ProvisionApplyOutcome {
            entries,
            access_key_limit: None,
        })
    })
    .await
}

/// Delete the root access key that set this account up, once the scoped user
/// has fully replaced it.
///
/// The onboarding guide tells the operator, twice, that Claria will create a
/// least-privilege user "and then delete the root access key automatically".
/// `bootstrap_account` has always had the step; nothing ever called it,
/// because the shipping path is `provision_apply`, which never learned the
/// source key's id. So the key stayed Active, against the promise and against
/// AWS's own guidance that an account should hold no root access key at all.
///
/// Non-fatal by design, matching `bootstrap_account`: setup that finishes with
/// a live root key is bad, setup that rolls back the account it just built
/// because of one is worse. The operator is told what to delete by hand.
///
/// Only a credential classified `Root` is touched. The classification comes
/// from a live `GetCallerIdentity` here rather than from anything the
/// frontend passed, because this call destroys a credential.
async fn retire_root_access_key(
    state: &State<'_, DesktopState>,
    region: &str,
    elevated: &CredentialSource,
    on_progress: &tauri::ipc::Channel<ProvisionerProgress>,
) {
    // Only an inline credential names a key that can be deleted. A profile,
    // the default chain, or an assumed role gives nothing to act on.
    let CredentialSource::Inline { access_key_id, .. } = elevated else {
        return;
    };

    let elevated_config = claria_desktop::aws::build_aws_config(region, elevated).await;
    match claria_provisioner::assess_credentials(&elevated_config).await {
        Ok(assessment) if assessment.credential_class == CredentialClass::Root => {}
        Ok(_) => return,
        Err(error) => {
            // Not fatal, and not silent: the promise went unkept and the
            // operator is the only one who can now keep it.
            tracing::warn!(
                error = %error,
                "could not classify the setup credentials, so the root access key was left alone"
            );
            let _ = on_progress.send(ProvisionerProgress::EscalationStep {
                label: ROOT_KEY_STEP.into(),
                status: "failed".into(),
            });
            return;
        }
    }

    let _ = on_progress.send(ProvisionerProgress::EscalationStep {
        label: ROOT_KEY_STEP.into(),
        status: "in_progress".into(),
    });

    if let Err(error) =
        claria_provisioner::delete_root_access_key(&elevated_config, access_key_id).await
    {
        tracing::warn!(
            error = %error,
            "failed to delete the root access key — the operator must remove it by hand"
        );
        let _ = on_progress.send(ProvisionerProgress::EscalationStep {
            label: ROOT_KEY_STEP.into(),
            status: "failed".into(),
        });
        return;
    }

    let _ = on_progress.send(ProvisionerProgress::EscalationStep {
        label: ROOT_KEY_STEP.into(),
        status: "done".into(),
    });

    // Destroying a credential is not something that may happen only in a ring
    // buffer. Best-effort, like every audit write — the key is already gone,
    // and failing the setup over the record of it would be the wrong trade.
    match CommandContext::new(state).await {
        Ok(ctx) => {
            let event = ctx
                .audit_event(
                    claria_storage::audit::actions::ROOT_ACCESS_KEY_DELETE,
                    "iam_access_key",
                    access_key_id.clone(),
                )
                .with_details(serde_json::json!({
                    "reason": "replaced by the scoped Claria IAM user during setup",
                }));
            ctx.record_audit(event).await;
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "root access key deleted but the audit event could not be recorded"
            );
        }
    }
}

/// The progress label for the root-key step, quoted in two places.
const ROOT_KEY_STEP: &str = "Deleting the root access key";
