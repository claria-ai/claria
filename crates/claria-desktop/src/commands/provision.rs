//! Credential assessment, bootstrap, and provisioner scan/apply commands.

use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CommandContext, CommandError, parse_uuid, run};
use crate::state::DesktopState;
use claria_desktop::config::{self, ClariaConfig, CredentialSource};
use claria_provisioner::{
    Action, CredentialScope, PlanEntry,
    account_setup::{
        AccessKeyInfo, BootstrapResult, BootstrapStep, CredentialAssessment, CredentialClass,
        StepStatus,
    },
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

/// Redacted [`BootstrapResult`] for the frontend: the minted secret access
/// key is persisted to the local config Rust-side and never returned.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct BootstrapOutcome {
    pub success: bool,
    pub steps: Vec<BootstrapStep>,
    pub account_id: Option<String>,
    /// Present when bootstrap minted scoped credentials.
    pub new_credentials: Option<NewCredentialsInfo>,
    pub error: Option<String>,
}

/// The non-secret half of freshly minted credentials.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct NewCredentialsInfo {
    pub access_key_id: String,
    pub iam_user_arn: String,
}

fn redact_bootstrap(result: BootstrapResult) -> BootstrapOutcome {
    BootstrapOutcome {
        success: result.success,
        steps: result.steps,
        account_id: result.account_id,
        new_credentials: result.new_credentials.map(|creds| NewCredentialsInfo {
            access_key_id: creds.access_key_id,
            iam_user_arn: creds.iam_user_arn,
        }),
        error: result.error,
    }
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
// Bootstrap command — orchestrates provisioner + config persistence
// ---------------------------------------------------------------------------

/// Run the full bootstrap flow: create a scoped IAM user and policy using
/// the operator's current (broad) credentials, then persist the new scoped
/// credentials to the local config.
///
/// The provisioner does all the IAM work and returns the new credentials.
/// We handle only the config write and in-memory state update.
#[tauri::command]
#[specta::specta]
pub async fn bootstrap_iam_user(
    state: State<'_, DesktopState>,
    region: String,
    system_name: String,
    root_access_key_id: String,
    root_secret_access_key: String,
    session_token: Option<String>,
    credential_class: CredentialClass,
) -> Result<BootstrapOutcome, String> {
    run("bootstrap_iam_user", async {
        // Build an SDK config from the raw credentials. These are held only in
        // memory — the desktop app never persists broad/root credentials to disk.
        // When a session_token is present, the credentials come from an
        // AssumeRole call (sub-account flow).
        let sdk_config = claria_desktop::aws::build_aws_config(
            &region,
            &CredentialSource::Inline {
                access_key_id: root_access_key_id.clone(),
                secret_access_key: root_secret_access_key,
                session_token,
            },
        )
        .await;

        // Delegate all IAM logic to the provisioner.
        let mut result = claria_provisioner::bootstrap_account(
            &sdk_config,
            &system_name,
            &root_access_key_id,
            credential_class,
        )
        .await;

        // If bootstrap succeeded, persist the new scoped credentials to config.
        if result.success
            && let Some(new_creds) = &result.new_credentials
        {
            let cfg = ClariaConfig {
                config_version: 0, // save_config stamps CURRENT_VERSION
                region: region.clone(),
                system_name,
                account_id: result.account_id.clone().unwrap_or_default(),
                created_at: jiff::Timestamp::now(),
                credentials: CredentialSource::Inline {
                    access_key_id: new_creds.access_key_id.clone(),
                    secret_access_key: new_creds.secret_access_key.clone(),
                    session_token: None,
                },
                preferred_model_id: None,
                cost_explorer_enabled: false,
                hourly_cost_data: false,
                prompt_caching_enabled: true,
                transcription: Default::default(),
                report_authoring: Default::default(),
            };

            if let Err(e) = config::save_config(&cfg) {
                // Bootstrap succeeded in AWS but we failed to write config
                // locally. The minted secret only lives in that config, so
                // tell the operator to free the key slot and retry rather
                // than echoing the secret back over IPC.
                let mut failed = result;
                failed.steps.push(claria_provisioner::BootstrapStep {
                    name: "write_config".to_string(),
                    status: StepStatus::Failed,
                    detail: Some(format!(
                        "Failed to write config: {e}. The new access key exists in AWS but \
                         could not be saved locally — delete it from the IAM user and run \
                         setup again."
                    )),
                });
                return Ok(redact_bootstrap(failed));
            }

            let mut guard = state.config.lock().await;
            *guard = Some(cfg);
            drop(guard);

            // ── Accept Bedrock model agreements ─────────────────────────
            //
            // Use the new scoped credentials to accept Marketplace agreements
            // for all available Claude models. This prevents the user from
            // hitting agreement errors when they first try to use chat.
            result.steps.push(claria_provisioner::BootstrapStep {
                name: "accept_model_agreements".to_string(),
                status: StepStatus::InProgress,
                detail: None,
            });

            let new_sdk_config = claria_desktop::aws::build_aws_config(
                &region,
                &CredentialSource::Inline {
                    access_key_id: new_creds.access_key_id.clone(),
                    secret_access_key: new_creds.secret_access_key.clone(),
                    session_token: None,
                },
            )
            .await;

            match claria_bedrock::chat::accept_all_model_agreements(&new_sdk_config).await {
                Ok(summary) => {
                    let detail = if summary.newly_accepted.is_empty() && summary.failed.is_empty() {
                        "All model agreements already accepted.".to_string()
                    } else {
                        let mut parts = Vec::new();
                        if !summary.newly_accepted.is_empty() {
                            parts.push(format!(
                                "Accepted {} model(s)",
                                summary.newly_accepted.len()
                            ));
                        }
                        if !summary.failed.is_empty() {
                            parts.push(format!("{} failed", summary.failed.len()));
                        }
                        parts.join(", ")
                    };

                    let step = result
                        .steps
                        .iter_mut()
                        .rfind(|s| s.name == "accept_model_agreements");
                    if let Some(s) = step {
                        s.status = StepStatus::Succeeded;
                        s.detail = Some(detail);
                    }
                }
                Err(e) => {
                    // Non-fatal: agreement acceptance failure shouldn't block
                    // the user from proceeding. They can accept later from chat.
                    let step = result
                        .steps
                        .iter_mut()
                        .rfind(|s| s.name == "accept_model_agreements");
                    if let Some(s) = step {
                        s.status = StepStatus::Failed;
                        s.detail = Some(format!(
                            "Non-fatal: {e}. You can accept model agreements later from the chat screen."
                        ));
                    }
                }
            }
        }

        Ok(redact_bootstrap(result))
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

/// Helper: scan all resources concurrently (up to 5 at a time), streaming
/// progress events via the channel. Returns plan entries in manifest order.
async fn scan_with_progress(
    syncers: &[Box<dyn claria_provisioner::ResourceSyncer>],
    prov_state: &claria_provisioner::ProvisionerState,
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

    entries.extend(claria_provisioner::find_orphans(syncers, prov_state));
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

        if entry.action == Action::Create {
            tracing::info!(addr = %addr, "creating resource");
            let result = syncer
                .create()
                .await
                .map_err(|e| e.with_resource(&entry.spec.label, &entry.spec.resource_name))?;
            prov_state.resources.insert(
                addr.clone(),
                claria_provisioner::state::ResourceState {
                    resource_type: entry.spec.resource_type.clone(),
                    resource_id: entry.spec.resource_name.clone(),
                    status: claria_provisioner::state::ResourceStatus::Created,
                    properties: result,
                },
            );
        } else {
            tracing::info!(addr = %addr, "updating resource");
            let result = syncer
                .update()
                .await
                .map_err(|e| e.with_resource(&entry.spec.label, &entry.spec.resource_name))?;
            if let Some(rs) = prov_state.resources.get_mut(&addr) {
                rs.status = claria_provisioner::state::ResourceStatus::Updated;
                rs.properties = result;
            }
        }
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
        if let Some(syncer) = syncer_map.get(&addr) {
            tracing::info!(addr = %addr, "destroying resource");
            syncer
                .destroy()
                .await
                .map_err(|e| e.with_resource(&entry.spec.label, &entry.spec.resource_name))?;
        }
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
        let manifest =
            claria_provisioner::build_manifest(&cfg.account_id, &cfg.system_name, &cfg.region);
        let syncers = claria_provisioner::build_syncers(&ctx.sdk_config, &manifest, None);
        let persistence = claria_provisioner::build_persistence(
            &ctx.sdk_config,
            &cfg.system_name,
            &cfg.account_id,
            &config::provisioner_state_dir(&cfg.system_name)?,
        )?;
        let prov_state = persistence.load().await?;

        scan_with_progress(&syncers, &prov_state, &on_progress).await
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
        let manifest =
            claria_provisioner::build_manifest(&cfg.account_id, &cfg.system_name, &cfg.region);
        let syncers = claria_provisioner::build_syncers(&ctx.sdk_config, &manifest, None);
        let persistence = claria_provisioner::build_persistence(
            &ctx.sdk_config,
            &cfg.system_name,
            &cfg.account_id,
            &config::provisioner_state_dir(&cfg.system_name)?,
        )?;

        let mut prov_state = persistence.load().await?;

        // Scan first (with progress).
        let entries = scan_with_progress(&syncers, &prov_state, &on_progress).await?;

        // Execute (with progress).
        execute_with_progress(
            &entries,
            &syncers,
            &mut prov_state,
            &persistence,
            &on_progress,
        )
        .await?;

        // Re-scan to show updated state (with progress).
        scan_with_progress(&syncers, &prov_state, &on_progress).await
    })
    .await
}

/// Destroy all managed resources. Returns nothing on success.
#[tauri::command]
#[specta::specta]
pub async fn destroy(state: State<'_, DesktopState>) -> Result<(), String> {
    run("destroy", async {
        let ctx = CommandContext::new(&state).await?;
        let cfg = &ctx.cfg;
        let manifest =
            claria_provisioner::build_manifest(&cfg.account_id, &cfg.system_name, &cfg.region);
        let syncers = claria_provisioner::build_syncers(&ctx.sdk_config, &manifest, None);
        let persistence = claria_provisioner::build_persistence(
            &ctx.sdk_config,
            &cfg.system_name,
            &cfg.account_id,
            &config::provisioner_state_dir(&cfg.system_name)?,
        )?;

        let mut prov_state = persistence.load().await?;
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

        let entries = scan_with_progress(&syncers, &prov_state, &on_progress).await?;

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

            // Scan elevated resources.
            let elevated_entries =
                scan_with_progress(&elevated_syncers, &prov_state, &on_progress).await?;

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

        let regular_config = if !config::has_config() {
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

        let regular_entries =
            scan_with_progress(&regular_syncers, &prov_state, &on_progress).await?;

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

        let entries = scan_with_progress(&all_syncers, &prov_state, &on_progress).await?;

        Ok(ProvisionApplyOutcome {
            entries,
            access_key_limit: None,
        })
    })
    .await
}
