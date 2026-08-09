use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::State;

use claria_desktop::config::{
    self, ClariaConfig, ConfigInfo, CredentialSource, ReportAuthoringPreferences,
    SyncedPreferences, TranscriptionPreferences,
};
use claria_provisioner::{
    Action, CredentialScope, PlanEntry,
    account_setup::{
    AccessKeyInfo, AssumeRoleResult, BootstrapResult, CredentialAssessment, CredentialClass,
    StepStatus,
    },
};

use claria_desktop::console::{ConsoleBuffer, ConsoleEntry};

use crate::state::DesktopState;

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
// Client + Chat types
// ---------------------------------------------------------------------------

pub use claria_desktop::{
    records::{ClientNameUpdate, ClientRecordDetails, ClientSummary},
    report_authoring::{
        EditorHistoryEntry, ReportBlockReferenceInput, ReportDraftEdit, ReportExportResult,
        ReportExportStatusView, ReportProposalDecision, ReportTurnProgressView, ReportTurnResponse,
        ReportWorkspaceView,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
}

/// Response from a chat message, including the persisted chat session ID
/// and per-turn token usage.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ChatResponse {
    pub chat_id: String,
    pub chat_name: String,
    pub content: String,
    pub usage: claria_core::models::turn_usage::TurnUsage,
}

/// Response from an infrastructure chat turn. Infra chat does not persist
/// history, but we still return token usage so the UI can display cost.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct InfraChatResponse {
    pub content: String,
    pub usage: claria_core::models::turn_usage::TurnUsage,
}

/// A single message in persisted chat history, including optional token
/// usage on assistant turns.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ChatHistoryDetailMessage {
    pub role: ChatRole,
    pub content: String,
    /// `Some` on assistant turns whose Converse response carried a usage
    /// block. `None` on user turns and on assistant turns from history
    /// written before per-turn usage tracking landed.
    pub usage: Option<claria_core::models::turn_usage::TurnUsage>,
}

/// Detail of a persisted chat session, returned when resuming a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ChatHistoryDetail {
    pub chat_id: String,
    pub name: String,
    pub model_id: String,
    pub messages: Vec<ChatHistoryDetailMessage>,
    pub created_at: String,
    pub updated_at: String,
}

/// Lightweight persisted-chat row for the Record screen history folder.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ChatHistorySummary {
    pub chat_id: String,
    pub filename: String,
    pub name: String,
    pub size: u64,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn has_config() -> Result<bool, String> {
    Ok(config::has_config())
}

#[tauri::command]
#[specta::specta]
pub async fn load_config(state: State<'_, DesktopState>) -> Result<ConfigInfo, String> {
    let mut cfg = config::load_config().map_err(|e| e.to_string())?;

    // Backfill account_id for configs saved before this field existed.
    if cfg.account_id.is_empty() {
        let sdk_config = cached_sdk_config(&state, &cfg).await;
        let sts = aws_sdk_sts::Client::new(&sdk_config);
        if let Ok(identity) = sts.get_caller_identity().send().await
            && let Some(account_id) = identity.account()
        {
            cfg.account_id = account_id.to_string();
            // Best-effort re-save so next load doesn't need STS again.
            let _ = config::save_config(&cfg);
        }
    }

    // Overlay synced preferences from S3 if any are stored there. First-launch
    // and any read failure (missing key, S3 unreachable) are non-fatal — the
    // local config remains authoritative.
    if !cfg.account_id.is_empty() {
        let sdk_config = cached_sdk_config(&state, &cfg).await;
        match read_cloud_preferences(&sdk_config, &cfg).await {
            Ok(Some(synced)) => {
                synced.apply_to_config(&mut cfg);
                tracing::debug!("applied synced preferences from S3");
            }
            Ok(None) => {
                // First boot against this bucket: seed the file with the local
                // values so every later read (here and on other machines)
                // finds it.
                let synced = SyncedPreferences::from_config(&cfg);
                match write_cloud_preferences(&sdk_config, &cfg, &synced).await {
                    Ok(()) => tracing::info!("initialized synced preferences in S3"),
                    Err(e) => tracing::warn!(
                        error = %e,
                        "failed to initialize synced preferences in S3"
                    ),
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to read cloud preferences; using local values");
            }
        }
    }

    let info = config::config_info(&cfg);

    let mut guard = state.config.lock().await;
    *guard = Some(cfg);

    Ok(info)
}

/// Read `_state/preferences.json` from S3. Returns `Ok(None)` when the object
/// doesn't exist (first launch, fresh provisioner); errors only on transport
/// failure or malformed JSON.
async fn read_cloud_preferences(
    sdk_config: &aws_config::SdkConfig,
    cfg: &ClariaConfig,
) -> Result<Option<SyncedPreferences>, String> {
    let s3 = claria_storage::client::from_config(sdk_config);
    let bucket = bucket_name(cfg);
    match claria_storage::objects::get_object(&s3, &bucket, claria_core::s3_keys::PREFERENCES).await
    {
        Ok(output) => {
            let synced: SyncedPreferences =
                serde_json::from_slice(&output.body).map_err(|e| e.to_string())?;
            synced.report_authoring.validate()?;
            Ok(Some(synced))
        }
        Err(claria_storage::error::StorageError::NotFound { .. }) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Write `_state/preferences.json` to S3.
async fn write_cloud_preferences(
    sdk_config: &aws_config::SdkConfig,
    cfg: &ClariaConfig,
    synced: &SyncedPreferences,
) -> Result<(), String> {
    let s3 = claria_storage::client::from_config(sdk_config);
    let bucket = bucket_name(cfg);
    let body = serde_json::to_vec_pretty(synced).map_err(|e| e.to_string())?;
    claria_storage::objects::put_object(
        &s3,
        &bucket,
        claria_core::s3_keys::PREFERENCES,
        body,
        Some("application/json"),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Persist `cfg` locally, mirror the synced subset to S3, and refresh the
/// in-memory copy.
///
/// Every command that changes a [`SyncedPreferences`] field must go through
/// here. `load_config` overlays the S3 copy onto the local config on each
/// call, so a local-only write is silently reverted by the next read — the
/// setting appears to save, survives on disk, and still comes back stale.
///
/// The local write happens first so the edit is not lost when S3 is
/// unreachable; the cloud failure is returned so the UI can say so.
async fn save_config_synced(
    state: &State<'_, DesktopState>,
    sdk_config: &aws_config::SdkConfig,
    cfg: ClariaConfig,
    what: &str,
) -> Result<(), String> {
    config::save_config(&cfg).map_err(|e| e.to_string())?;

    let synced = SyncedPreferences::from_config(&cfg);
    let cloud = write_cloud_preferences(sdk_config, &cfg, &synced)
        .await
        .map_err(|e| format!("{what} saved locally but cloud sync failed: {e}"));

    let mut guard = state.config.lock().await;
    *guard = Some(cfg);
    drop(guard);

    cloud
}

/// Save the clinician's preferences (synced subset) to both the local config
/// file and `_state/preferences.json` in S3. Bubbles S3-write failures so the
/// frontend can show a partial-save warning.
#[tauri::command]
#[specta::specta]
pub async fn save_preferences(
    state: State<'_, DesktopState>,
    preferred_model_id: Option<String>,
    cost_explorer_enabled: bool,
    hourly_cost_data: bool,
    prompt_caching_enabled: bool,
    transcription: TranscriptionPreferences,
    report_authoring: ReportAuthoringPreferences,
) -> Result<ConfigInfo, String> {
    let (mut cfg, sdk_config) = load_sdk_config(&state).await?;

    cfg.preferred_model_id = preferred_model_id;
    cfg.cost_explorer_enabled = cost_explorer_enabled;
    cfg.hourly_cost_data = hourly_cost_data;
    cfg.prompt_caching_enabled = prompt_caching_enabled;
    cfg.transcription = transcription;
    report_authoring.validate()?;
    cfg.report_authoring = report_authoring;

    // Persist locally first so we don't lose the user's edit if S3 is down.
    config::save_config(&cfg).map_err(|e| e.to_string())?;

    let synced = SyncedPreferences::from_config(&cfg);
    write_cloud_preferences(&sdk_config, &cfg, &synced)
        .await
        .map_err(|e| format!("preferences saved locally but cloud sync failed: {e}"))?;

    let info = config::config_info(&cfg);
    let mut guard = state.config.lock().await;
    *guard = Some(cfg);
    Ok(info)
}

/// Re-fetch synced preferences from S3 and overlay onto the in-memory config.
/// Used by the Preferences page on entry so users on the editing machine see
/// the latest cloud state without an app restart.
#[tauri::command]
#[specta::specta]
pub async fn fetch_cloud_preferences(state: State<'_, DesktopState>) -> Result<ConfigInfo, String> {
    let (mut cfg, sdk_config) = load_sdk_config(&state).await?;
    if let Some(synced) = read_cloud_preferences(&sdk_config, &cfg).await? {
        synced.apply_to_config(&mut cfg);
        config::save_config(&cfg).map_err(|e| e.to_string())?;
    }
    let info = config::config_info(&cfg);
    let mut guard = state.config.lock().await;
    *guard = Some(cfg);
    Ok(info)
}

#[tauri::command]
#[specta::specta]
pub async fn save_config(
    state: State<'_, DesktopState>,
    region: String,
    system_name: String,
    account_id: String,
    credentials: CredentialSource,
) -> Result<(), String> {
    let cfg = ClariaConfig {
        config_version: 0, // save_config stamps CURRENT_VERSION
        region,
        system_name,
        account_id,
        created_at: jiff::Timestamp::now(),
        credentials,
        preferred_model_id: None,
        cost_explorer_enabled: false,
        hourly_cost_data: false,
        prompt_caching_enabled: true,
        transcription: Default::default(),
        report_authoring: Default::default(),
    };

    config::save_config(&cfg).map_err(|e| e.to_string())?;

    let mut guard = state.config.lock().await;
    *guard = Some(cfg);

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_config(state: State<'_, DesktopState>) -> Result<(), String> {
    config::delete_config().map_err(|e| e.to_string())?;

    let mut guard = state.config.lock().await;
    *guard = None;

    Ok(())
}

/// Set the clinician's preferred chat model.
///
/// Loads the current config, updates `preferred_model_id`, and saves. Pass
/// `None` to clear the preference (fall back to the first available model).
///
/// `preferred_model_id` is part of [`SyncedPreferences`], and `load_config`
/// overlays that S3 copy onto the local config on every call. Writing only the
/// local file would therefore be undone by the very next `load_config` — the
/// pick would survive on disk but the app would keep reading the stale cloud
/// value. The cloud write is not optional for this field.
#[tauri::command]
#[specta::specta]
pub async fn set_preferred_model(
    state: State<'_, DesktopState>,
    model_id: Option<String>,
) -> Result<(), String> {
    let (mut cfg, sdk_config) = load_sdk_config(&state).await?;
    cfg.preferred_model_id = model_id;
    save_config_synced(&state, &sdk_config, cfg, "preferred model").await
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
    region: String,
    credentials: CredentialSource,
) -> Result<CredentialAssessment, String> {
    let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;
    claria_provisioner::assess_credentials(&sdk_config)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Role assumption command — for sub-account (Persona A) flow
// ---------------------------------------------------------------------------

/// Assume a role in an AWS sub-account using parent-account credentials.
///
/// The operator provides their parent-account credentials and the sub-account
/// details. We call STS AssumeRole and return temporary credentials that can
/// be used with `assess_credentials` and `bootstrap_iam_user` to set up a
/// dedicated IAM user in the sub-account.
///
/// The temporary credentials are never persisted to disk.
#[tauri::command]
#[specta::specta]
pub async fn assume_role(
    region: String,
    credentials: CredentialSource,
    account_id: String,
    role_name: String,
) -> Result<AssumeRoleResult, String> {
    let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;

    let role_arn = claria_provisioner::build_role_arn(&account_id, &role_name);

    claria_provisioner::assume_role(&sdk_config, &role_arn, None)
        .await
        .map_err(|e| e.to_string())
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
    region: String,
    credentials: CredentialSource,
) -> Result<Vec<AccessKeyInfo>, String> {
    let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;
    claria_provisioner::list_user_access_keys(&sdk_config)
        .await
        .map_err(|e| e.to_string())
}

/// Delete one access key belonging to the `claria-admin` IAM user.
///
/// Called after the operator picks a key to remove to make room for a
/// fresh one during bootstrap.
#[tauri::command]
#[specta::specta]
pub async fn delete_user_access_key(
    region: String,
    credentials: CredentialSource,
    access_key_id: String,
) -> Result<(), String> {
    let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;
    claria_provisioner::delete_user_access_key(&sdk_config, &access_key_id)
        .await
        .map_err(|e| e.to_string())
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
) -> Result<BootstrapResult, String> {
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
                // locally. Return a modified result so the frontend can
                // show the new credentials and let the operator save them
                // manually.
                let mut failed = result;
                failed.steps.push(claria_provisioner::BootstrapStep {
                    name: "write_config".to_string(),
                    status: StepStatus::Failed,
                    detail: Some(format!("Failed to write config: {e}")),
                });
                return Ok(failed);
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
                        s.status = if summary.failed.is_empty() {
                            StepStatus::Succeeded
                        } else {
                            // Non-fatal: some agreements failed but bootstrap itself worked.
                            StepStatus::Succeeded
                        };
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

    Ok(result)
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
    let (cfg, _) = load_sdk_config(&state).await?;

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
    .await
    .map_err(|e| e.to_string())?;

    let _ = on_progress.send(ProvisionerProgress::EscalationStep {
        label: "Updating IAM policy document".into(),
        status: "done".into(),
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// Provisioner commands — scan, plan, provision, destroy
// ---------------------------------------------------------------------------

/// Helper: load the saved config and build an SDK config from it.
///
/// If the in-memory state is empty, attempts to load from disk first.
/// Returns `(ClariaConfig, SdkConfig)`. Errors if no config is saved yet.
///
/// The built `SdkConfig` is cached in state and reused while the region and
/// credentials it was built from are unchanged: its pooled HTTP connector is
/// what keeps connections warm across commands, and rebuilding it per command
/// pays DNS/TCP/TLS setup on every S3 call.
pub(crate) async fn load_sdk_config(
    state: &State<'_, DesktopState>,
) -> Result<(ClariaConfig, aws_config::SdkConfig), String> {
    let mut guard = state.config.lock().await;

    // Auto-load from disk if the in-memory state hasn't been populated yet.
    if guard.is_none()
        && let Ok(cfg) = config::load_config()
    {
        *guard = Some(cfg);
    }

    let cfg = guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "No config loaded. Complete setup first.".to_string())?;
    drop(guard);

    let sdk_config = cached_sdk_config(state, &cfg).await;
    Ok((cfg, sdk_config))
}

/// Helper: the cached `SdkConfig` for `cfg`'s region and credentials,
/// building and caching it on miss. Reuse is what keeps the pooled HTTP
/// connections warm across commands.
async fn cached_sdk_config(
    state: &State<'_, DesktopState>,
    cfg: &ClariaConfig,
) -> aws_config::SdkConfig {
    let mut sdk_guard = state.sdk_config.lock().await;
    if let Some(cached) = sdk_guard.as_ref()
        && cached.region == cfg.region
        && cached.credentials == cfg.credentials
    {
        return cached.sdk_config.clone();
    }

    let sdk_config = claria_desktop::aws::build_aws_config(&cfg.region, &cfg.credentials).await;
    *sdk_guard = Some(crate::state::CachedSdkConfig {
        region: cfg.region.clone(),
        credentials: cfg.credentials.clone(),
        sdk_config: sdk_config.clone(),
    });
    sdk_config
}

/// Helper: scan all resources concurrently (up to 5 at a time), streaming
/// progress events via the channel. Returns plan entries in manifest order.
async fn scan_with_progress(
    syncers: &[Box<dyn claria_provisioner::ResourceSyncer>],
    prov_state: &claria_provisioner::ProvisionerState,
    on_progress: &tauri::ipc::Channel<ProvisionerProgress>,
) -> Result<Vec<PlanEntry>, String> {
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
        let actual = actual_result.map_err(|e| e.to_string())?;
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
) -> Result<(), String> {
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
            format!(
                "no syncer for {} {}",
                addr.resource_type, addr.resource_name
            )
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
            let result = syncer.create().await.map_err(|e| {
                e.with_resource(&entry.spec.label, &entry.spec.resource_name)
                    .to_string()
            })?;
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
            let result = syncer.update().await.map_err(|e| {
                e.with_resource(&entry.spec.label, &entry.spec.resource_name)
                    .to_string()
            })?;
            if let Some(rs) = prov_state.resources.get_mut(&addr) {
                rs.status = claria_provisioner::state::ResourceStatus::Updated;
                rs.properties = result;
            }
        }
        persistence
            .flush(prov_state)
            .await
            .map_err(|e| e.to_string())?;

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
            syncer.destroy().await.map_err(|e| {
                e.with_resource(&entry.spec.label, &entry.spec.resource_name)
                    .to_string()
            })?;
        }
        prov_state.resources.remove(&addr);
        persistence
            .flush(prov_state)
            .await
            .map_err(|e| e.to_string())?;
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
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let manifest =
        claria_provisioner::build_manifest(&cfg.account_id, &cfg.system_name, &cfg.region);
    let syncers = claria_provisioner::build_syncers(&sdk_config, &manifest, None);
    let persistence =
        claria_provisioner::build_persistence(&sdk_config, &cfg.system_name, &cfg.account_id)
    .map_err(|e| e.to_string())?;
    let prov_state = persistence.load().await.map_err(|e| e.to_string())?;

    scan_with_progress(&syncers, &prov_state, &on_progress).await
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
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let manifest =
        claria_provisioner::build_manifest(&cfg.account_id, &cfg.system_name, &cfg.region);
    let syncers = claria_provisioner::build_syncers(&sdk_config, &manifest, None);
    let persistence =
        claria_provisioner::build_persistence(&sdk_config, &cfg.system_name, &cfg.account_id)
    .map_err(|e| e.to_string())?;

    let mut prov_state = persistence.load().await.map_err(|e| e.to_string())?;

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
}

/// Destroy all managed resources. Returns nothing on success.
#[tauri::command]
#[specta::specta]
pub async fn destroy(state: State<'_, DesktopState>) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let manifest =
        claria_provisioner::build_manifest(&cfg.account_id, &cfg.system_name, &cfg.region);
    let syncers = claria_provisioner::build_syncers(&sdk_config, &manifest, None);
    let persistence =
        claria_provisioner::build_persistence(&sdk_config, &cfg.system_name, &cfg.account_id)
    .map_err(|e| e.to_string())?;

    let mut prov_state = persistence.load().await.map_err(|e| e.to_string())?;
    claria_provisioner::destroy_all(&syncers, &mut prov_state, &persistence)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete the provisioner state file (local + S3) so the next scan starts fresh.
///
/// Use this when state is incompatible with the current version of Claria.
/// AWS resources are not affected — the next scan will re-discover them.
#[tauri::command]
#[specta::specta]
pub async fn reset_provisioner_state(state: State<'_, DesktopState>) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let persistence =
        claria_provisioner::build_persistence(&sdk_config, &cfg.system_name, &cfg.account_id)
    .map_err(|e| e.to_string())?;
    persistence.delete().await.map_err(|e| e.to_string())
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
    region: String,
    system_name: String,
    credentials: CredentialSource,
    on_progress: tauri::ipc::Channel<ProvisionerProgress>,
) -> Result<ProvisionScanResult, String> {
    let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;

    // Resolve account ID via STS.
    let identity = claria_provisioner::account_setup::get_caller_identity(&sdk_config)
        .await
        .map_err(|e| e.to_string())?;

    let manifest = claria_provisioner::build_manifest(&identity.account_id, &system_name, &region);
    let syncers = claria_provisioner::build_syncers(&sdk_config, &manifest, None);

    // Try to load state; fall back to empty state if persistence isn't set up yet.
    let prov_state = match claria_provisioner::build_persistence(
        &sdk_config,
        &system_name,
        &identity.account_id,
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
    credentials: CredentialSource,
    elevated_credentials: Option<CredentialSource>,
    on_progress: tauri::ipc::Channel<ProvisionerProgress>,
) -> Result<ProvisionApplyOutcome, String> {
    let sdk_config = claria_desktop::aws::build_aws_config(&region, &credentials).await;

    let identity = claria_provisioner::account_setup::get_caller_identity(&sdk_config)
        .await
        .map_err(|e| e.to_string())?;

    let manifest = claria_provisioner::build_manifest(&identity.account_id, &system_name, &region);

    // We need persistence that can work even before the S3 bucket exists.
    // For local-only state during bootstrap, build persistence with the
    // elevated config (which can at least do local writes).
    let persistence =
        claria_provisioner::build_persistence(&sdk_config, &system_name, &identity.account_id)
    .map_err(|e| e.to_string())?;

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
        let elevated_config = claria_desktop::aws::build_aws_config(&region, elevated_creds).await;

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
        let elevated_creds = elevated_credentials
            .as_ref()
            .ok_or("Elevated credentials required to create access key for new IAM user")?;
        let elevated_config = claria_desktop::aws::build_aws_config(&region, elevated_creds).await;

        let _ = on_progress.send(ProvisionerProgress::EscalationStep {
            label: "Creating access key for claria-admin".into(),
            status: "in_progress".into(),
        });

        let (key_id, secret) = match claria_provisioner::create_access_key(&elevated_config).await {
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
            Err(e) => return Err(e.to_string()),
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
            .await
            .map_err(|e| e.to_string())?;

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

        config::save_config(&cfg).map_err(|e| e.to_string())?;

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

    let regular_entries = scan_with_progress(&regular_syncers, &prov_state, &on_progress).await?;

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
        )
        .map_err(|e| e.to_string())?;

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
}

// ---------------------------------------------------------------------------
// Client commands — CRUD backed by S3
// ---------------------------------------------------------------------------

/// Helper: derive bucket name from saved config.
pub(crate) fn bucket_name(cfg: &ClariaConfig) -> String {
    claria_core::s3_keys::bucket_name(&cfg.account_id, &cfg.system_name)
}

/// Helper: record an audit event against the bucket derived from `cfg`.
///
/// See [`claria_desktop::audit::record`] for why this cannot fail the caller.
pub(crate) async fn record_audit(
    sdk_config: &aws_config::SdkConfig,
    cfg: &ClariaConfig,
    event: claria_audit::events::AuditEvent,
) {
    claria_desktop::audit::record(sdk_config, &bucket_name(cfg), event).await
}

/// List all client records from S3.
///
/// Loads each `clients/{id}.json` object, deserializes the Client, and
/// returns summaries sorted by most recently created first.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(count = tracing::field::Empty))]
pub async fn list_clients(state: State<'_, DesktopState>) -> Result<Vec<ClientSummary>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let clients =
        claria_desktop::records::list_client_summaries(&s3, &bucket, &state.record_cache).await?;

    tracing::Span::current().record("count", clients.len() as u64);

    Ok(clients)
}

/// Create a new client record in S3.
#[tauri::command]
#[specta::specta]
pub async fn create_client(
    state: State<'_, DesktopState>,
    name: String,
) -> Result<ClientSummary, String> {
    let name = claria_desktop::records::validate_client_name(&name)?;
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id = uuid::Uuid::new_v4();
    let now = jiff::Timestamp::now();
    let client = claria_core::models::client::Client {
        id,
        name: name.clone(),
        created_at: now,
        updated_at: now,
    };

    let body = serde_json::to_vec_pretty(&client).map_err(|e| e.to_string())?;
    let key = claria_core::s3_keys::client(id);

    claria_storage::objects::put_object(&s3, &bucket, &key, body, Some("application/json"))
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!(client_id = %id, name = %name, "client record created");

    Ok(ClientSummary {
        id: id.to_string(),
        name,
        created_at: now.to_string(),
    })
}

/// Load editable metadata, storage statistics, and name history for one client.
#[tauri::command]
#[specta::specta]
pub async fn get_client_record_details(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<ClientRecordDetails, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    claria_desktop::records::get_client_record_details(&s3, &bucket, id).await
}

/// Update a client's display name with optimistic concurrency control.
#[tauri::command]
#[specta::specta]
pub async fn update_client_name(
    state: State<'_, DesktopState>,
    client_id: String,
    name: String,
) -> Result<ClientNameUpdate, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let update = claria_desktop::records::update_client_name(&s3, &bucket, id, &name).await?;
    tracing::info!(client_id = %id, "client record renamed");
    Ok(update)
}

/// Delete a client and all associated data through the retryable,
/// compensating lifecycle library.
#[tauri::command]
#[specta::specta]
pub async fn delete_client(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let outcome = claria_client_lifecycle::delete_client(&s3, &bucket, id)
        .await
        .map_err(|error| error.to_string())?;
    tracing::info!(
        client_id = %id,
        deleted_records = outcome.deleted_records,
        deleted_report_objects = outcome.deleted_report_objects,
        "client deleted"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Writing — separate opt-in report workflow
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn load_report_workspace(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<ReportWorkspaceView, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let workspace = claria_report_authoring::load_report_workspace(&s3, &bucket, client_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(claria_desktop::report_authoring::workspace_view(&workspace))
}

/// Return the current persisted writing session for the Record screen's
/// Editor History folder without creating a new workspace.
#[tauri::command]
#[specta::specta]
pub async fn list_editor_history(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<Vec<EditorHistoryEntry>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let workspace = claria_report_authoring::find_report_workspace(&s3, &bucket, client_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(workspace
        .as_ref()
        .map(claria_desktop::report_authoring::editor_history_entry)
        .into_iter()
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn rename_report_session(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    name: String,
) -> Result<ReportWorkspaceView, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let report_id = report_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let workspace =
        claria_report_authoring::rename_report_session(&s3, &bucket, client_id, report_id, &name)
            .await
            .map_err(|error| error.to_string())?;
    let workspace = claria_desktop::report_authoring::workspace_view(&workspace);

    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "report_session_renamed",
            "report",
            workspace.report_id.clone(),
            cfg.account_id.clone(),
        )
        .with_details(serde_json::json!({ "client_id": client_id.to_string() })),
    )
    .await;

    Ok(workspace)
}

#[tauri::command]
#[specta::specta]
pub async fn save_report_draft(
    state: State<'_, DesktopState>,
    client_id: String,
    expected_revision: u64,
    draft: ReportDraftEdit,
) -> Result<ReportWorkspaceView, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let content = claria_desktop::report_authoring::content_from_edit(draft)?;
    let workspace = claria_report_authoring::save_report_draft(
        &s3,
        &bucket,
        client_id,
        expected_revision,
        content,
    )
    .await
    .map_err(|error| error.to_string())?;
    let workspace = claria_desktop::report_authoring::workspace_view(&workspace);

    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "report_draft_saved",
            "report",
            workspace.report_id.clone(),
            cfg.account_id.clone(),
        )
        .with_details(serde_json::json!({
            "client_id": client_id.to_string(),
            "report_id": workspace.report_id,
            "revision": workspace.draft.revision,
            "section_count": workspace.draft.content.sections.len()
        })),
    )
    .await;

    Ok(workspace)
}

#[tauri::command]
#[specta::specta]
pub async fn send_report_message(
    state: State<'_, DesktopState>,
    client_id: String,
    expected_revision: u64,
    model_id: String,
    instruction: String,
    references: Vec<ReportBlockReferenceInput>,
    on_progress: tauri::ipc::Channel<ReportTurnProgressView>,
) -> Result<ReportTurnResponse, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let references = references
        .into_iter()
        .map(ReportBlockReferenceInput::into_domain)
        .collect::<Result<Vec<_>, _>>()?;
    let limits = cfg.report_authoring.limits()?;
    let progress = |event: claria_report_authoring::ReportTurnProgress| {
        let _ = on_progress.send(event.into());
    };
    let result = claria_report_authoring::send_report_message(
        &sdk_config,
        &s3,
        &bucket,
        client_id,
        expected_revision,
        &model_id,
        claria_report_authoring::ReportMessageRequest::new(&instruction)
            .with_references(&references)
            .with_limits(limits)
            .with_progress(&progress),
    )
    .await;

    match result {
        Ok(outcome) => {
            let attempt = outcome.attempt.clone();
            let response = claria_desktop::report_authoring::turn_response_view(outcome);
            record_audit(
                &sdk_config,
                &cfg,
                claria_audit::events::AuditEvent::new(
                    "report_tool_turn_succeeded",
                    "report",
                    attempt.report_id.to_string(),
                    cfg.account_id.clone(),
                )
                .with_details(serde_json::json!({
                    "status": "succeeded",
                    "client_id": attempt.client_id.to_string(),
                    "report_id": attempt.report_id.to_string(),
                    "attempt_id": attempt.attempt_id.to_string(),
                    "turn_id": response.turn_id,
                    "proposal_id": response.proposal_id,
                    "revision": response.workspace.draft.revision,
                    "model_id": attempt.model_id,
                    "converse_calls": attempt.converse_calls,
                    "tool_uses": attempt.tool_uses,
                    "usage_complete": attempt.usage_complete,
                    "input_tokens": attempt.usage.input_tokens,
                    "output_tokens": attempt.usage.output_tokens,
                    "cache_read_input_tokens": attempt.usage.cache_read_input_tokens,
                    "cache_write_input_tokens": attempt.usage.cache_write_input_tokens,
                    "cost_usd": attempt.usage.cost_usd,
                    "pricing_version": attempt.usage.pricing_version
                })),
            )
            .await;
            Ok(response)
        }
        Err(error) => {
            let attempt = error.attempt().cloned();
            let resource_id = attempt.as_ref().map_or_else(
                || client_id.to_string(),
                |value| value.report_id.to_string(),
            );
            record_audit(
                &sdk_config,
                &cfg,
                claria_audit::events::AuditEvent::new(
                    "report_tool_turn_failed",
                    "report",
                    resource_id,
                    cfg.account_id.clone(),
                )
                .with_details(serde_json::json!({
                    "status": "failed",
                    "client_id": client_id.to_string(),
                    "report_id": attempt.as_ref().map(|value| value.report_id.to_string()),
                    "attempt_id": attempt.as_ref().map(|value| value.attempt_id.to_string()),
                    "model_id": model_id,
                    "failure_code": error.failure_code(),
                    "converse_calls": attempt.as_ref().map_or(0, |value| value.converse_calls),
                    "tool_uses": attempt.as_ref().map_or(0, |value| value.tool_uses),
                    "usage_complete": attempt.as_ref().is_none_or(|value| value.usage_complete),
                    "input_tokens": attempt.as_ref().map_or(0, |value| value.usage.input_tokens),
                    "output_tokens": attempt.as_ref().map_or(0, |value| value.usage.output_tokens),
                    "cache_read_input_tokens": attempt.as_ref().map_or(0, |value| value.usage.cache_read_input_tokens),
                    "cache_write_input_tokens": attempt.as_ref().map_or(0, |value| value.usage.cache_write_input_tokens),
                    "cost_usd": attempt.as_ref().map_or(0.0, |value| value.usage.cost_usd),
                    "pricing_version": attempt.as_ref().map_or(0, |value| value.usage.pricing_version)
                })),
            )
            .await;
            Err(error.to_string())
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn resolve_report_proposal(
    state: State<'_, DesktopState>,
    client_id: String,
    proposal_id: String,
    decision: ReportProposalDecision,
) -> Result<ReportWorkspaceView, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let proposal_id = proposal_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let action = match decision {
        ReportProposalDecision::Accept => "report_proposal_accepted",
        ReportProposalDecision::Reject => "report_proposal_rejected",
    };
    let workspace = claria_report_authoring::resolve_report_proposal(
        &s3,
        &bucket,
        client_id,
        proposal_id,
        decision.into(),
    )
    .await
    .map_err(|error| error.to_string())?;
    let workspace = claria_desktop::report_authoring::workspace_view(&workspace);

    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            action,
            "report",
            workspace.report_id.clone(),
            cfg.account_id.clone(),
        )
        .with_details(serde_json::json!({
            "client_id": client_id.to_string(),
            "report_id": workspace.report_id,
            "proposal_id": proposal_id.to_string(),
            "resulting_revision": workspace.draft.revision,
            "section_count": workspace.draft.content.sections.len()
        })),
    )
    .await;

    Ok(workspace)
}

#[tauri::command]
#[specta::specta]
pub async fn export_report_docx(
    state: State<'_, DesktopState>,
    client_id: String,
    report_id: String,
    expected_revision: u64,
) -> Result<ReportExportResult, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let report_id = report_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let snapshot = claria_report_authoring::load_export_snapshot(
        &s3,
        &bucket,
        client_id,
        report_id,
        expected_revision,
    )
    .await
    .map_err(|error| error.to_string())?;
    let bytes = if let Some(template) = snapshot.template_source.as_deref() {
        claria_docx::render_report_with_template(template, &snapshot.draft)
    } else {
        claria_docx::render_report(&snapshot.draft)
    }
    .map_err(|error| error.to_string())?;
    let draft = snapshot.draft;
    let filename = claria_report_authoring::suggested_docx_filename(&draft.content.title);
    // Use the asynchronous dialog implementation. In particular, macOS must
    // schedule NSSavePanel work on the main thread; opening the synchronous
    // dialog after async S3 work can otherwise return as canceled repeatedly.
    let selected = rfd::AsyncFileDialog::new()
        .set_title("Export report to Word")
        .set_file_name(filename)
        .add_filter("Word documents", &["docx"])
        .save_file()
        .await;
    let Some(selected) = selected else {
        let attempted_at = jiff::Timestamp::now();
        let status_persisted = claria_report_authoring::record_report_export(
            &s3,
            &bucket,
            client_id,
            report_id,
            draft.revision,
            claria_core::models::report::ReportExportStatus::Canceled,
        )
        .await
        .is_ok();
        return Ok(ReportExportResult {
            exported: false,
            report_id: report_id.to_string(),
            revision: draft.revision,
            status: ReportExportStatusView::Canceled,
            attempted_at: attempted_at.to_string(),
            status_persisted,
        });
    };
    let mut path = selected.path().to_path_buf();
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("docx"))
    {
        path.set_extension("docx");
    }
    // The selected local path is intentionally never logged or audited.
    if let Err(error) = claria_desktop::local_export::write_private_atomic(&path, &bytes) {
        let _ = claria_report_authoring::record_report_export(
            &s3,
            &bucket,
            client_id,
            report_id,
            draft.revision,
            claria_core::models::report::ReportExportStatus::Failed,
        )
        .await;
        return Err(error.to_string());
    }
    let attempted_at = jiff::Timestamp::now();
    let status_persisted = claria_report_authoring::record_report_export(
        &s3,
        &bucket,
        client_id,
        report_id,
        draft.revision,
        claria_core::models::report::ReportExportStatus::Exported,
    )
    .await
    .is_ok();

    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "report_docx_exported",
            "report",
            report_id.to_string(),
            cfg.account_id.clone(),
        )
        .with_details(serde_json::json!({
            "client_id": client_id.to_string(),
            "report_id": report_id.to_string(),
            "revision": draft.revision,
            "section_count": draft.content.sections.len(),
            "destination": "local_unmanaged_storage"
        })),
    )
    .await;

    Ok(ReportExportResult {
        exported: true,
        report_id: report_id.to_string(),
        revision: draft.revision,
        status: ReportExportStatusView::Exported,
        attempted_at: attempted_at.to_string(),
        status_persisted,
    })
}

// ---------------------------------------------------------------------------
// Record file commands — files attached to a client record
// ---------------------------------------------------------------------------

/// A file in a client's record (S3 object metadata).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RecordFile {
    pub filename: String,
    pub size: i32,
    pub uploaded_at: Option<String>,
}

/// The Bedrock model ID used for document text extraction.
///
/// Uses a Claude Sonnet inference profile — good quality at lower cost.
const EXTRACTION_MODEL_ID: &str = "us.anthropic.claude-sonnet-4-20250514-v1:0";

/// The Bedrock model ID used for per-segment transcript translation.
///
/// Pinned to Claude Sonnet 4.6: handles specialized vocabulary (drug names,
/// anatomy, dosage phrases) more reliably than Haiku, at a cost rounding-error
/// compared to the Transcribe spend per session. Same rationale as
/// [`EXTRACTION_MODEL_ID`] — internal operations get pinned to a sensible model
/// rather than exposing yet another preference knob.
const TRANSLATION_MODEL_ID: &str = "us.anthropic.claude-sonnet-4-6";

/// List files in a client's record, excluding sidecar `.text` files.
///
/// `prefix` narrows the listing to filenames starting with it, mapped to the
/// S3 ListObjectsV2 `Prefix` parameter (`records/{id}/{prefix}`).
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(client_id = %client_id, count = tracing::field::Empty))]
pub async fn list_record_files(
    state: State<'_, DesktopState>,
    client_id: String,
    prefix: Option<String>,
) -> Result<Vec<RecordFile>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let records_prefix = claria_core::s3_keys::client_records_prefix(id);
    let list_prefix = match prefix.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => claria_core::s3_keys::client_records_search_prefix(id, p),
        None => records_prefix.clone(),
    };

    let objects = claria_storage::objects::list_objects_with_metadata(&s3, &bucket, &list_prefix)
        .await
        .map_err(|e| e.to_string())?;

    // Collect all keys into a set so we can check for base files when filtering sidecars.
    let all_keys: std::collections::HashSet<&str> =
        objects.iter().map(|o| o.key.as_str()).collect();

    let files: Vec<RecordFile> = objects
        .iter()
        .filter(|obj| !claria_core::s3_keys::is_hidden_sidecar(&obj.key, &all_keys))
        .filter_map(|obj| {
            // Strip the records prefix to get just the filename.
            let filename = obj.key.strip_prefix(&records_prefix)?;
            if filename.is_empty() {
                return None;
            }
            Some(RecordFile {
                filename: filename.to_string(),
                size: obj.size as i32,
                uploaded_at: obj.last_modified.clone(),
            })
        })
        .collect();

    tracing::Span::current().record("count", files.len() as u64);

    Ok(files)
}

/// Filenames in a client's record whose readable text contains `query`,
/// case-insensitively.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(client_id = %client_id, count = tracing::field::Empty))]
pub async fn search_record_contents(
    state: State<'_, DesktopState>,
    client_id: String,
    query: String,
) -> Result<Vec<String>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let matches = claria_desktop::records::search_record_contents(
        &s3,
        &bucket,
        id,
        &state.record_cache,
        &query,
    )
    .await?;

    tracing::Span::current().record("count", matches.len() as u64);

    Ok(matches)
}

/// Upload a file to a client's record from a local file path.
///
/// If the file is a PDF or DOCX, a sidecar `.text` file is generated
/// via Bedrock document text extraction and uploaded alongside.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(
        client_id = %client_id,
        filename = tracing::field::Empty,
        bytes = tracing::field::Empty
    )
)]
pub async fn upload_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    file_path: String,
) -> Result<RecordFile, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let path = std::path::Path::new(&file_path);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Invalid file path".to_string())?;

    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;
    let file_size = bytes.len() as i32;

    let span = tracing::Span::current();
    span.record("filename", filename);
    span.record("bytes", bytes.len() as u64);

    // Determine content type from extension.
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let content_type = match extension.as_str() {
        "pdf" => Some("application/pdf"),
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "doc" => Some("application/msword"),
        "txt" => Some("text/plain"),
        "csv" => Some("text/csv"),
        "html" | "htm" => Some("text/html"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "mp3" => Some("audio/mpeg"),
        "mp4" | "m4a" => Some("audio/mp4"),
        "wav" => Some("audio/wav"),
        "flac" => Some("audio/flac"),
        "ogg" => Some("audio/ogg"),
        "amr" => Some("audio/amr"),
        "webm" => Some("audio/webm"),
        _ => None,
    };

    // Upload the original file.
    let key = claria_core::s3_keys::client_record_file(id, filename);
    claria_storage::objects::put_object(&s3, &bucket, &key, bytes.clone(), content_type)
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!(client_id = %id, filename, "record file uploaded");

    // Generate sidecar text extraction for supported document types.
    if let Some(format) = claria_bedrock::extract::document_format_for_extension(&extension) {
        let sidecar_key = format!("{key}.text");
        let extraction_prompt = load_prompt(&s3, &bucket, "pdf-extraction").await?;
        match claria_bedrock::extract::extract_document_text(
            &sdk_config,
            EXTRACTION_MODEL_ID,
            &bytes,
            filename,
            format,
            &extraction_prompt,
        )
        .await
        {
            Ok((text, usage)) => {
                claria_storage::objects::put_object(
                    &s3,
                    &bucket,
                    &sidecar_key,
                    text.into_bytes(),
                    Some("text/plain"),
                )
                .await
                .map_err(|e| e.to_string())?;

                record_audit(
                    &sdk_config,
                    &cfg,
                    claria_audit::events::AuditEvent::new(
                        "extract_document_text",
                        "record_file",
                        filename,
                        cfg.account_id.clone(),
                    )
                    .with_details(serde_json::json!({
                        "client_id": id.to_string(),
                        "model_id": usage.model_id,
                        "input_tokens": usage.input_tokens,
                        "output_tokens": usage.output_tokens,
                        "cache_read_input_tokens": usage.cache_read_input_tokens,
                        "cache_write_input_tokens": usage.cache_write_input_tokens,
                        "cost_usd": usage.cost_usd,
                        "pricing_version": usage.pricing_version,
                    })),
                )
                .await;

                tracing::info!(client_id = %id, filename, "sidecar text extraction uploaded");
            }
            Err(e) => {
                // Non-fatal: the original file is already uploaded.
                tracing::warn!(
                    client_id = %id,
                    filename,
                    error = %e,
                    "sidecar text extraction failed"
                );
            }
        }
    } else if let Some(media_format) = claria_transcribe::media_format_for_extension(&extension) {
        let sidecar_key = format!("{key}.text");
        // Drag-drop uses saved preferences as-is. The wizard's separate
        // command (`upload_record_file_with_options`) is the override path.
        let options = build_transcribe_options(&cfg.transcription, None);
        let translate = cfg.transcription.translate_to_english;

        match claria_transcribe::transcribe_audio_with_options(
            &sdk_config,
            &bucket,
            &key,
            media_format,
            &options,
        )
        .await
        {
            Ok(mut result) => {
                maybe_translate(&sdk_config, &cfg, &mut result, translate).await;
                let body = claria_transcribe::format_transcript_body(&result);
                claria_storage::objects::put_object(
                    &s3,
                    &bucket,
                    &sidecar_key,
                    body.into_bytes(),
                    Some("text/plain"),
                )
                .await
                .map_err(|e| e.to_string())?;

                tracing::info!(client_id = %id, filename, "sidecar audio transcription uploaded");
            }
            Err(e) => {
                // Non-fatal: the original file is already uploaded.
                tracing::warn!(
                    client_id = %id,
                    filename,
                    error = %e,
                    "sidecar audio transcription failed"
                );
            }
        }
    }

    Ok(RecordFile {
        filename: filename.to_string(),
        size: file_size,
        uploaded_at: Some(jiff::Timestamp::now().to_string()),
    })
}

/// Map per-clinician `TranscriptionPreferences` + per-file overrides into the
/// `TranscribeOptions` shape the library crate expects.
fn build_transcribe_options(
    prefs: &TranscriptionPreferences,
    overrides: Option<TranscribeOptionsOverrides>,
) -> claria_transcribe::TranscribeOptions {
    let lang_pref = overrides
        .as_ref()
        .and_then(|o| o.language)
        .unwrap_or(prefs.default_language);
    let language = match lang_pref {
        config::TranscriptionLanguage::English => claria_transcribe::LanguageMode::English,
        config::TranscriptionLanguage::Spanish => claria_transcribe::LanguageMode::Spanish,
        config::TranscriptionLanguage::Mixed => claria_transcribe::LanguageMode::Mixed,
    };

    let speaker_count = overrides
        .as_ref()
        .and_then(|o| o.speaker_count)
        .unwrap_or(prefs.default_speaker_count);

    let speakers = match overrides.as_ref().and_then(|o| o.speaker_mode) {
        Some(SpeakerMode::None) => claria_transcribe::SpeakerHandling::None,
        Some(SpeakerMode::Channels) => claria_transcribe::SpeakerHandling::Channels,
        Some(SpeakerMode::Diarize) | None => match speaker_count {
            0 | 1 => claria_transcribe::SpeakerHandling::None,
            n => claria_transcribe::SpeakerHandling::Diarize { max: n },
        },
    };

    let use_medical = overrides
        .as_ref()
        .and_then(|o| o.use_medical_for_english)
        .unwrap_or(prefs.use_medical_for_english);
    let engine = if use_medical {
        claria_transcribe::TranscriptionEngine::Medical
    } else {
        claria_transcribe::TranscriptionEngine::Standard
    };

    claria_transcribe::TranscribeOptions {
        language,
        speakers,
        engine,
    }
}

/// Per-file overrides for the wizard flow. Each field is optional so the
/// frontend only sends what the user actually changed; everything else falls
/// back to the saved preferences. Uses the `TranscriptionLanguage` type from
/// our config crate (specta-typed) rather than the library's `LanguageMode` —
/// the wrapper keeps the TS binding inside the desktop crate's surface.
#[derive(Debug, Clone, Deserialize, specta::Type)]
pub struct TranscribeOptionsOverrides {
    #[serde(default)]
    pub language: Option<config::TranscriptionLanguage>,
    #[serde(default)]
    pub speaker_mode: Option<SpeakerMode>,
    #[serde(default)]
    pub speaker_count: Option<u8>,
    #[serde(default)]
    pub use_medical_for_english: Option<bool>,
    /// When set, overrides `prefs.translate_to_english` for this single file.
    #[serde(default)]
    pub translate_to_english: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerMode {
    None,
    Diarize,
    Channels,
}

/// Translate non-English segments in-place if translation is enabled.
async fn maybe_translate(
    sdk_config: &aws_config::SdkConfig,
    cfg: &ClariaConfig,
    result: &mut claria_transcribe::TranscriptResult,
    translate: bool,
) {
    if !translate {
        return;
    }
    let model_id = TRANSLATION_MODEL_ID;

    let requests: Vec<claria_bedrock::translate::TranslationRequest> = result
        .segments
        .iter()
        .enumerate()
        .filter_map(|(idx, seg)| {
            let lang = seg.language_code.as_deref()?;
            if lang == "en-US" || lang.starts_with("en-") || seg.text.trim().is_empty() {
                return None;
            }
            Some(claria_bedrock::translate::TranslationRequest {
                index: idx,
                language_code: lang.to_string(),
                source_text: seg.text.clone(),
            })
        })
        .collect();

    if requests.is_empty() {
        return;
    }

    match claria_bedrock::translate::translate_segments(sdk_config, model_id, &requests).await {
        Ok((outputs, usage)) => {
            for output in &outputs {
                if let Some(seg) = result.segments.get_mut(output.index) {
                    seg.translation = Some(output.translation.clone());
                }
            }
            record_audit(
                sdk_config,
                cfg,
                claria_audit::events::AuditEvent::new(
                    "translate_transcript",
                    "transcript",
                    "",
                    cfg.account_id.clone(),
                )
                .with_details(serde_json::json!({
                    "segment_count": outputs.len(),
                    "model_id": usage.model_id,
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                    "cost_usd": usage.cost_usd,
                    "pricing_version": usage.pricing_version,
                })),
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(error = %e, "translation failed; sidecar will be written without translations");
        }
    }
}

/// Upload an audio file and transcribe with the wizard's per-file options.
///
/// Mirrors `upload_record_file` but skips the legacy single-language path —
/// always goes through the new structured transcribe + optional Bedrock
/// translation. The `.text` sidecar contains the rendered headered body.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(client_id = %client_id))]
pub async fn upload_record_file_with_options(
    state: State<'_, DesktopState>,
    client_id: String,
    file_path: String,
    overrides: Option<TranscribeOptionsOverrides>,
) -> Result<RecordFile, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let path = std::path::Path::new(&file_path);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Invalid file path".to_string())?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let media_format = claria_transcribe::media_format_for_extension(&extension)
        .ok_or_else(|| format!("Unsupported audio format: .{extension}"))?;

    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;
    let file_size = bytes.len() as i32;

    let content_type = match extension.as_str() {
        "mp3" => Some("audio/mpeg"),
        "mp4" | "m4a" => Some("audio/mp4"),
        "wav" => Some("audio/wav"),
        "flac" => Some("audio/flac"),
        "ogg" => Some("audio/ogg"),
        "amr" => Some("audio/amr"),
        "webm" => Some("audio/webm"),
        _ => None,
    };

    let key = claria_core::s3_keys::client_record_file(id, filename);
    claria_storage::objects::put_object(&s3, &bucket, &key, bytes, content_type)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(client_id = %id, filename, "record file uploaded (wizard path)");

    let translate = overrides
        .as_ref()
        .and_then(|o| o.translate_to_english)
        .unwrap_or(cfg.transcription.translate_to_english);
    let options = build_transcribe_options(&cfg.transcription, overrides);

    let sidecar_key = format!("{key}.text");
    let mut result = claria_transcribe::transcribe_audio_with_options(
        &sdk_config,
        &bucket,
        &key,
        media_format,
        &options,
    )
            .await
            .map_err(|e| e.to_string())?;

    maybe_translate(&sdk_config, &cfg, &mut result, translate).await;

    let body = claria_transcribe::format_transcript_body(&result);
    claria_storage::objects::put_object(
        &s3,
        &bucket,
        &sidecar_key,
        body.into_bytes(),
        Some("text/plain"),
    )
    .await
    .map_err(|e| e.to_string())?;

    tracing::info!(client_id = %id, filename, "wizard transcription complete");

    Ok(RecordFile {
        filename: filename.to_string(),
        size: file_size,
        uploaded_at: Some(jiff::Timestamp::now().to_string()),
    })
}

/// Open a native file picker scoped to supported audio formats. Returns the
/// absolute path the user chose, or `None` if they cancelled.
///
/// Used by the transcription wizard so we can keep a real file picker on the
/// wizard surface (avoiding the geometry-sensitive drag-target controls flagged
/// in [feedback-ui-low-dexterity]).
#[tauri::command]
#[specta::specta]
pub fn pick_audio_file() -> Result<Option<String>, String> {
    let path = rfd::FileDialog::new()
        .set_title("Choose an audio file to transcribe")
        .add_filter(
            "Audio",
            &["mp3", "m4a", "mp4", "wav", "flac", "ogg", "amr", "webm"],
        )
        .pick_file();
    Ok(path.and_then(|p| p.to_str().map(|s| s.to_string())))
}

/// Save edits to the transcript sidecar. S3 versioning preserves every prior
/// body, including the Transcribe-generated v1 — clinicians restore any past
/// version (or the original) via the standard `list_file_versions` /
/// `restore_file_version` flow, which the frontend routes to the `.text`
/// sidecar for audio files.
#[tauri::command]
#[specta::specta]
pub async fn save_transcript_edits(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    body: String,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let sidecar_key = format!(
        "{}.text",
        claria_core::s3_keys::client_record_file(id, &filename)
    );

    claria_storage::objects::put_object(
        &s3,
        &bucket,
        &sidecar_key,
        body.into_bytes(),
        Some("text/plain"),
    )
    .await
    .map_err(|e| e.to_string())?;

    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "save_transcript_edits",
            "transcript",
            &filename,
            cfg.account_id.clone(),
        )
        .with_details(serde_json::json!({ "client_id": id.to_string() })),
    )
    .await;

    Ok(())
}

/// Delete a file from a client's record, including its sidecar if present.
#[tauri::command]
#[specta::specta]
pub async fn delete_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let key = claria_core::s3_keys::client_record_file(id, &filename);

    // Delete the original file.
    claria_storage::objects::delete_object(&s3, &bucket, &key)
        .await
        .map_err(|e| e.to_string())?;

    // Best-effort delete of the sidecar — but only for file types that
    // produce one (PDF, DOCX, audio). Plain text files never have a sidecar,
    // and deleting a non-existent key on a versioned bucket creates a phantom
    // delete marker.
    if !filename.ends_with(".txt") {
        let sidecar_key = format!("{key}.text");
        let _ = claria_storage::objects::delete_object(&s3, &bucket, &sidecar_key).await;
    }

    tracing::info!(client_id = %id, filename, "record file deleted");

    Ok(())
}

/// Get the text content for a record file.
///
/// For plain text files (`.txt`), returns the file content directly.
/// For other files, returns the `.text` sidecar content if available.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(client_id = %client_id, filename = %filename))]
pub async fn get_record_file_text(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
) -> Result<String, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let key = claria_core::s3_keys::client_record_file(id, &filename);

    // Plain text files: return the file content directly.
    if filename.ends_with(".txt") {
        return match claria_storage::objects::get_object(&s3, &bucket, &key).await {
            Ok(output) => String::from_utf8(output.body).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
    }

    // Other files: look for the `.text` sidecar.
    let sidecar_key = format!("{key}.text");

    match claria_storage::objects::get_object(&s3, &bucket, &sidecar_key).await {
        Ok(output) => String::from_utf8(output.body).map_err(|e| e.to_string()),
        Err(claria_storage::error::StorageError::NotFound { .. }) => {
            Ok("No text extraction available for this file.".to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Create a plain text file in a client's record.
///
/// Writes the given content as a `.txt` file directly to S3. If the filename
/// doesn't already end in `.txt`, it is appended.
#[tauri::command]
#[specta::specta]
pub async fn create_text_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    content: String,
) -> Result<RecordFile, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    // Ensure the filename ends with .txt.
    let filename = if filename.ends_with(".txt") {
        filename
    } else {
        format!("{filename}.txt")
    };

    let bytes = content.into_bytes();
    let file_size = bytes.len() as i32;

    let key = claria_core::s3_keys::client_record_file(id, &filename);
    claria_storage::objects::put_object(&s3, &bucket, &key, bytes, Some("text/plain"))
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!(client_id = %id, filename, "text record file created");

    Ok(RecordFile {
        filename,
        size: file_size,
        uploaded_at: Some(jiff::Timestamp::now().to_string()),
    })
}

/// Update the content of an existing plain text file in a client's record.
#[tauri::command]
#[specta::specta]
pub async fn update_text_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    content: String,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let key = claria_core::s3_keys::client_record_file(id, &filename);
    claria_storage::objects::put_object(
        &s3,
        &bucket,
        &key,
        content.into_bytes(),
        Some("text/plain"),
    )
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!(client_id = %id, filename, "text record file updated");

    Ok(())
}

// ---------------------------------------------------------------------------
// Record context — text content for chat context injection
// ---------------------------------------------------------------------------

/// A record file with its readable text content, for chat context.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RecordContext {
    pub filename: String,
    pub text: String,
}

/// Load text content for all record files belonging to a client.
///
/// For `.txt` files, returns the file content directly. For PDF/DOCX,
/// returns the `.text` sidecar content if available. Files with no
/// readable text are omitted.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(client_id = %client_id, files = tracing::field::Empty))]
pub async fn list_record_context(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<Vec<RecordContext>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let texts =
        claria_desktop::records::fetch_record_texts(&s3, &bucket, id, &state.record_cache).await?;

    // Include all files — those without extracted text get an empty string
    // so the frontend can show them as context pills and offer re-extraction.
    let context_files: Vec<RecordContext> = texts
        .into_iter()
        .map(|(filename, text)| RecordContext {
            filename,
            text: text.unwrap_or_default(),
        })
        .collect();

    tracing::Span::current().record("files", context_files.len() as u64);

    Ok(context_files)
}

/// Re-run text extraction for a single record file.
///
/// Downloads the original file from S3, runs Bedrock document extraction
/// (or audio transcription for audio files), uploads the `.text` sidecar,
/// and returns the updated `RecordContext` with the extracted text.
#[tauri::command]
#[specta::specta]
pub async fn extract_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
) -> Result<RecordContext, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let key = claria_core::s3_keys::client_record_file(id, &filename);

    let extension = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let sidecar_key = format!("{key}.text");

    let text = if let Some(format) =
        claria_bedrock::extract::document_format_for_extension(&extension)
    {
        // Document extraction (PDF, DOCX).
        let output = claria_storage::objects::get_object(&s3, &bucket, &key)
            .await
            .map_err(|e| e.to_string())?;
        let extraction_prompt = load_prompt(&s3, &bucket, "pdf-extraction").await?;
        let (text, usage) = claria_bedrock::extract::extract_document_text(
            &sdk_config,
            EXTRACTION_MODEL_ID,
            &output.body,
            &filename,
            format,
            &extraction_prompt,
        )
        .await
        .map_err(|e| e.to_string())?;

        claria_storage::objects::put_object(
            &s3,
            &bucket,
            &sidecar_key,
            text.clone().into_bytes(),
            Some("text/plain"),
        )
        .await
        .map_err(|e| e.to_string())?;

        record_audit(
            &sdk_config,
            &cfg,
            claria_audit::events::AuditEvent::new(
                "extract_document_text",
                "record_file",
                &filename,
                cfg.account_id.clone(),
            )
            .with_details(serde_json::json!({
                "client_id": id.to_string(),
                "model_id": usage.model_id,
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_read_input_tokens": usage.cache_read_input_tokens,
                "cache_write_input_tokens": usage.cache_write_input_tokens,
                "cost_usd": usage.cost_usd,
                "pricing_version": usage.pricing_version,
            })),
        )
        .await;

        text
    } else if let Some(media_format) = claria_transcribe::media_format_for_extension(&extension) {
        // Audio transcription using saved preferences (re-extract path).
        let options = build_transcribe_options(&cfg.transcription, None);
        let mut result = claria_transcribe::transcribe_audio_with_options(
            &sdk_config,
            &bucket,
            &key,
            media_format,
            &options,
        )
        .await
        .map_err(|e| e.to_string())?;
        maybe_translate(
            &sdk_config,
            &cfg,
            &mut result,
            cfg.transcription.translate_to_english,
        )
        .await;
        let text = claria_transcribe::format_transcript_body(&result);

        claria_storage::objects::put_object(
            &s3,
            &bucket,
            &sidecar_key,
            text.clone().into_bytes(),
            Some("text/plain"),
        )
        .await
        .map_err(|e| e.to_string())?;

        text
    } else {
        return Err(format!("unsupported file type for extraction: {filename}"));
    };

    tracing::info!(client_id = %id, filename, "re-extracted text for record file");

    Ok(RecordContext { filename, text })
}

/// Helper: load all record context for a client, converting to bedrock types.
async fn load_record_context(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: &str,
    cache: &claria_desktop::record_cache::RecordCache,
) -> Result<Vec<claria_bedrock::context::ContextFile>, String> {
    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let texts = claria_desktop::records::fetch_record_texts(s3, bucket, id, cache).await?;

    Ok(texts
        .into_iter()
        .filter_map(|(filename, text)| {
            text.map(|text| claria_bedrock::context::ContextFile { filename, text })
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Chat commands — delegates to claria-bedrock
// ---------------------------------------------------------------------------

/// Specta type mirroring `claria_bedrock::chat::ChatModel`.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ChatModel {
    pub model_id: String,
    pub name: String,
}

/// Default system prompt, used when no custom prompt has been saved to S3.
const DEFAULT_SYSTEM_PROMPT: &str = "\
You are a clinical assistant helping a psychologist set up a new client record. \
Help gather relevant intake information such as the client's presenting concerns, \
referral source, relevant history, and initial observations. \
Be professional, empathetic, and concise. Ask clarifying questions when needed. \
Do not provide diagnoses or treatment recommendations — your role is to help \
organize and document the intake information.";

/// Resolve a prompt name to its S3 key and hardcoded default text.
///
/// Returns `(s3_key, legacy_key, default_text)`. The `legacy_key` is `Some`
/// only for the system prompt which was previously stored at the bucket root.
fn resolve_prompt(
    name: &str,
) -> Result<(&'static str, Option<&'static str>, &'static str), String> {
    match name {
        "system-prompt" => Ok((
            claria_core::s3_keys::SYSTEM_PROMPT,
            Some(claria_core::s3_keys::LEGACY_SYSTEM_PROMPT),
            DEFAULT_SYSTEM_PROMPT,
        )),
        "pdf-extraction" => Ok((
            claria_core::s3_keys::EXTRACTION_PROMPT,
            None,
            claria_bedrock::extract::DEFAULT_EXTRACTION_PROMPT,
        )),
        _ => Err(format!("unknown prompt name: {name}")),
    }
}

/// Load a prompt from S3 by name, falling back to the legacy path and then the
/// hardcoded default.
async fn load_prompt(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    prompt_name: &str,
) -> Result<String, String> {
    let (key, legacy_key, default_text) = resolve_prompt(prompt_name)?;

    // Try the canonical claria-prompts/ key first.
    match claria_storage::objects::get_object(s3, bucket, key).await {
        Ok(output) => return String::from_utf8(output.body).map_err(|e| e.to_string()),
        Err(claria_storage::error::StorageError::NotFound { .. }) => {}
        Err(e) => return Err(e.to_string()),
    }

    // Fall back to the legacy key if one exists (system-prompt.md at bucket root).
    // When found, migrate it to the new path and delete the legacy key.
    if let Some(legacy) = legacy_key {
        match claria_storage::objects::get_object(s3, bucket, legacy).await {
            Ok(output) => {
                let text = String::from_utf8(output.body).map_err(|e| e.to_string())?;

                // Copy to the new claria-prompts/ path.
                if let Err(e) = claria_storage::objects::put_object(
                    s3,
                    bucket,
                    key,
                    text.as_bytes().to_vec(),
                    Some("text/markdown"),
                )
                .await
                {
                    tracing::warn!(legacy, key, error = %e, "failed to migrate legacy prompt");
                    return Ok(text);
                }

                // Remove the legacy key.
                if let Err(e) = claria_storage::objects::delete_object(s3, bucket, legacy).await {
                    tracing::warn!(legacy, error = %e, "failed to delete legacy prompt after migration");
                }

                tracing::info!(legacy, key, "migrated legacy prompt to claria-prompts/");
                return Ok(text);
            }
            Err(claria_storage::error::StorageError::NotFound { .. }) => {}
            Err(e) => return Err(e.to_string()),
        }
    }

    Ok(default_text.to_string())
}

/// List available Anthropic Claude models for chat.
///
/// Queries Bedrock for system-defined inference profiles and returns
/// those matching Anthropic Claude models.
#[tauri::command]
#[specta::specta]
pub async fn list_chat_models(state: State<'_, DesktopState>) -> Result<Vec<ChatModel>, String> {
    let (_cfg, sdk_config) = load_sdk_config(&state).await?;
    let models = claria_bedrock::chat::list_chat_models(&sdk_config)
        .await
        .map_err(|e| e.to_string())?;

    Ok(models
        .into_iter()
        .map(|m| ChatModel {
            model_id: m.model_id,
            name: m.name,
        })
        .collect())
}

const MAX_CHAT_HISTORY_BYTES: u64 = 5 * 1024 * 1024;
const MAX_CHAT_NAME_CHARACTERS: usize = 120;

fn normalized_chat_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a chat name.".to_string());
    }
    if name.chars().count() > MAX_CHAT_NAME_CHARACTERS {
        return Err(format!(
            "Chat names may contain at most {MAX_CHAT_NAME_CHARACTERS} characters."
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("Chat names cannot contain control characters.".to_string());
    }
    Ok(name.to_string())
}

async fn chat_history_rows(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
) -> Result<Vec<(claria_core::models::chat_history::ChatHistory, u64)>, String> {
    let prefix = claria_core::s3_keys::chat_history_prefix(client_id);
    let objects = claria_storage::objects::list_objects_with_metadata(s3, bucket, &prefix)
        .await
        .map_err(|error| error.to_string())?;
    let mut rows = Vec::new();
    for object in objects {
        if !object.key.ends_with(".json") {
            continue;
        }
        let output = claria_storage::objects::get_object_bounded(
            s3,
            bucket,
            &object.key,
            MAX_CHAT_HISTORY_BYTES,
        )
        .await
        .map_err(|error| error.to_string())?;
        let history: claria_core::models::chat_history::ChatHistory =
            serde_json::from_slice(&output.body).map_err(|error| error.to_string())?;
        if history.client_id != client_id {
            return Err("A stored chat history belongs to another client.".to_string());
        }
        rows.push((history, u64::try_from(object.size).unwrap_or(0)));
    }
    rows.sort_by_key(|(history, _)| history.created_at);
    Ok(rows)
}

async fn stored_chat_history(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
    chat_id: uuid::Uuid,
) -> Result<(claria_core::models::chat_history::ChatHistory, String), String> {
    let key = claria_core::s3_keys::chat_history(client_id, chat_id);
    let output =
        claria_storage::objects::get_object_bounded(s3, bucket, &key, MAX_CHAT_HISTORY_BYTES)
            .await
            .map_err(|error| error.to_string())?;
    let history: claria_core::models::chat_history::ChatHistory =
        serde_json::from_slice(&output.body).map_err(|error| error.to_string())?;
    if history.client_id != client_id || history.id != chat_id {
        return Err("The stored chat history has mismatched identifiers.".to_string());
    }
    let etag = output
        .etag
        .filter(|etag| !etag.trim().is_empty())
        .ok_or_else(|| "The stored chat history is missing a concurrency token.".to_string())?;
    Ok((history, etag))
}

fn chat_history_summary(
    history: &claria_core::models::chat_history::ChatHistory,
    size: u64,
    ordinal: usize,
) -> ChatHistorySummary {
    ChatHistorySummary {
        chat_id: history.id.to_string(),
        filename: format!("chat-history/{}.json", history.id),
        name: if history.name.trim().is_empty() {
            format!("Chat ({ordinal})")
        } else {
            history.name.clone()
        },
        size,
        updated_at: history.updated_at.to_string(),
    }
}

fn next_chat_history_name(
    rows: &[(claria_core::models::chat_history::ChatHistory, u64)],
) -> String {
    (1..)
        .map(|ordinal| format!("Chat ({ordinal})"))
        .find(|candidate| {
            rows.iter().enumerate().all(|(index, (history, _))| {
                let existing = if history.name.trim().is_empty() {
                    format!("Chat ({})", index + 1)
                } else {
                    history.name.clone()
                };
                existing != *candidate
            })
        })
        .expect("an unused chat ordinal exists")
}

fn chat_history_detail(
    history: claria_core::models::chat_history::ChatHistory,
    fallback_name: String,
) -> ChatHistoryDetail {
    let name = if history.name.trim().is_empty() {
        fallback_name
    } else {
        history.name.clone()
    };
    ChatHistoryDetail {
        chat_id: history.id.to_string(),
        name,
        model_id: history.model_id,
        messages: history
            .messages
            .into_iter()
            .map(|message| ChatHistoryDetailMessage {
                role: match message.role {
                    claria_core::models::chat_history::ChatHistoryRole::User => ChatRole::User,
                    claria_core::models::chat_history::ChatHistoryRole::Assistant => {
                        ChatRole::Assistant
                    }
                },
                content: message.content,
                usage: message.usage,
            })
            .collect(),
        created_at: history.created_at.to_string(),
        updated_at: history.updated_at.to_string(),
    }
}

/// Send a chat message to Bedrock and return the assistant's response.
///
/// The frontend maintains the full conversation history and sends it
/// with each request so the model has context. The system prompt is
/// fetched from S3 on each call so edits take effect immediately.
/// Record context (text from the client's files) is loaded from S3
/// and prepended to the system prompt.
///
/// After each successful exchange, the full conversation is persisted
/// to S3 under `records/{client_id}/chat-history/{chat_id}.json`.
/// The `chat_id` is generated on the first message and returned so the
/// frontend can pass it back on subsequent calls.
#[tauri::command]
#[specta::specta]
// Timing span logs ids, model, and turn count — never chat text (PHI).
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(client_id = %client_id, model_id = %model_id, turns = messages.len())
)]
pub async fn chat_message(
    state: State<'_, DesktopState>,
    client_id: String,
    model_id: String,
    messages: Vec<ChatMessage>,
    chat_id: Option<String>,
    chat_name: Option<String>,
    context_filenames: Vec<String>,
) -> Result<ChatResponse, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let client_uuid: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let now = jiff::Timestamp::now();
    let (chat_uuid, chat_name, created_at, expected_etag) = match &chat_id {
        Some(id) => {
            let chat_uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
            let (history, etag) = stored_chat_history(&s3, &bucket, client_uuid, chat_uuid).await?;
            let name = if history.name.trim().is_empty() {
                let rows = chat_history_rows(&s3, &bucket, client_uuid).await?;
                rows.iter()
                    .position(|(candidate, _)| candidate.id == chat_uuid)
                    .map_or_else(
                        || "Chat (1)".to_string(),
                        |index| format!("Chat ({})", index + 1),
                    )
            } else {
                history.name
            };
            (chat_uuid, name, history.created_at, Some(etag))
        }
        None => {
            let name = match chat_name {
                Some(name) => normalized_chat_name(&name)?,
                None => {
                    let rows = chat_history_rows(&s3, &bucket, client_uuid).await?;
                    next_chat_history_name(&rows)
                }
            };
            (uuid::Uuid::new_v4(), name, now, None)
        }
    };

    let system_prompt = load_prompt(&s3, &bucket, "system-prompt").await?;

    // Load record context and filter to the frontend's active set.
    let all_files = load_record_context(&s3, &bucket, &client_id, &state.record_cache).await?;
    let context_files: Vec<_> = if context_filenames.is_empty() {
        all_files
    } else {
        let allowed: std::collections::HashSet<&str> =
            context_filenames.iter().map(|s| s.as_str()).collect();
        all_files
            .into_iter()
            .filter(|f| allowed.contains(f.filename.as_str()))
            .collect()
    };
    let context_block = claria_bedrock::context::build_context_block(&context_files);
    let full_prompt = if context_block.is_empty() {
        system_prompt
    } else {
        format!("{context_block}\n\n{system_prompt}")
    };

    let bedrock_messages: Vec<claria_bedrock::chat::ChatMessage> = messages
        .iter()
        .map(|m| claria_bedrock::chat::ChatMessage {
            role: match m.role {
                ChatRole::User => claria_bedrock::chat::ChatRole::User,
                ChatRole::Assistant => claria_bedrock::chat::ChatRole::Assistant,
            },
            content: m.content.clone(),
        })
        .collect();

    let cache_strategy = build_cache_strategy(&cfg, &model_id);

    let (response_text, usage) = claria_bedrock::chat::chat_converse(
        &sdk_config,
        &model_id,
        &full_prompt,
        &bedrock_messages,
        cache_strategy,
    )
    .await
    .map_err(|e| e.to_string())?;

    // Emit a per-turn audit event with token usage in details. UUIDs only;
    // never the message content.
    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "chat_message",
            "client",
            client_uuid.to_string(),
            cfg.account_id.clone(),
        )
        .with_details(serde_json::json!({
            "chat_id": chat_uuid.to_string(),
            "model_id": usage.model_id,
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "cache_read_input_tokens": usage.cache_read_input_tokens,
            "cache_write_input_tokens": usage.cache_write_input_tokens,
            "cost_usd": usage.cost_usd,
            "pricing_version": usage.pricing_version,
        })),
    )
    .await;

    // Build the full message history including the new assistant response.
    let updated_at = jiff::Timestamp::now();
    let mut history_messages: Vec<claria_core::models::chat_history::ChatHistoryMessage> = messages
        .iter()
        .map(|m| claria_core::models::chat_history::ChatHistoryMessage {
            role: match m.role {
                ChatRole::User => claria_core::models::chat_history::ChatHistoryRole::User,
                ChatRole::Assistant => {
                    claria_core::models::chat_history::ChatHistoryRole::Assistant
                }
            },
            content: m.content.clone(),
            timestamp: updated_at,
            usage: None,
        })
        .collect();
    history_messages.push(claria_core::models::chat_history::ChatHistoryMessage {
        role: claria_core::models::chat_history::ChatHistoryRole::Assistant,
        content: response_text.clone(),
        timestamp: updated_at,
        usage: Some(usage.clone()),
    });

    let history = claria_core::models::chat_history::ChatHistory {
        id: chat_uuid,
        client_id: client_uuid,
        name: chat_name.clone(),
        model_id: model_id.clone(),
        messages: history_messages,
        created_at,
        updated_at,
    };

    // Best-effort upload — don't fail the chat if persistence fails.
    let key = claria_core::s3_keys::chat_history(client_uuid, chat_uuid);
    match serde_json::to_vec_pretty(&history) {
        Ok(body) => {
            let persisted = if let Some(etag) = expected_etag.as_deref() {
                claria_storage::objects::put_object_if_match(
                    &s3,
                    &bucket,
                    &key,
                    body,
                    Some("application/json"),
                    etag,
                )
                .await
            } else {
                claria_storage::objects::put_object_if_none_match(
                    &s3,
                    &bucket,
                    &key,
                    body,
                    Some("application/json"),
                )
                .await
            };
            if let Err(e) = persisted {
                tracing::warn!(
                    chat_id = %chat_uuid,
                    client_id = %client_uuid,
                    error = %e,
                    "failed to persist chat history"
                );
            } else {
                tracing::info!(
                    chat_id = %chat_uuid,
                    client_id = %client_uuid,
                    "chat history persisted"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                chat_id = %chat_uuid,
                error = %e,
                "failed to serialize chat history"
            );
        }
    }

    Ok(ChatResponse {
        chat_id: chat_uuid.to_string(),
        chat_name,
        content: response_text,
        usage,
    })
}

/// Chat about infrastructure — no history persistence.
///
/// The frontend passes pre-scanned `plan_entries` so we don't re-scan AWS
/// on every message. We build a rich system prompt explaining Claria's
/// operating model and the current infrastructure state, then call Bedrock.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(model_id = %model_id, turns = messages.len()))]
pub async fn infra_chat(
    state: State<'_, DesktopState>,
    model_id: String,
    messages: Vec<ChatMessage>,
    plan_entries: Vec<PlanEntry>,
) -> Result<InfraChatResponse, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;

    let system_prompt = build_infra_system_prompt(&plan_entries);

    let bedrock_messages: Vec<claria_bedrock::chat::ChatMessage> = messages
        .iter()
        .map(|m| claria_bedrock::chat::ChatMessage {
            role: match m.role {
                ChatRole::User => claria_bedrock::chat::ChatRole::User,
                ChatRole::Assistant => claria_bedrock::chat::ChatRole::Assistant,
            },
            content: m.content.clone(),
        })
        .collect();

    let cache_strategy = build_cache_strategy(&cfg, &model_id);

    let (content, usage) = claria_bedrock::chat::chat_converse(
        &sdk_config,
        &model_id,
        &system_prompt,
        &bedrock_messages,
        cache_strategy,
    )
    .await
    .map_err(|e| e.to_string())?;

    // Audit the infra-chat turn against the AWS account_id (no per-client
    // resource here).
    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "infra_chat",
            "infrastructure",
            "infra",
            cfg.account_id.clone(),
        )
        .with_details(serde_json::json!({
            "model_id": usage.model_id,
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "cache_read_input_tokens": usage.cache_read_input_tokens,
            "cache_write_input_tokens": usage.cache_write_input_tokens,
            "cost_usd": usage.cost_usd,
            "pricing_version": usage.pricing_version,
        })),
    )
    .await;

    Ok(InfraChatResponse { content, usage })
}

/// Build the full system prompt for infrastructure chat from plan entries.
/// Derive a [`claria_bedrock::chat::CacheStrategy`] from config + the
/// inference profile we're about to invoke.
///
/// Caching is honoured for Claude Sonnet 4 and Opus 4 (5-min TTL — Sonnet
/// 4.5 / Opus 4.5 with 1h TTL is a future Phase). Haiku 3.5 also supports
/// it. Models we don't recognise default to no cache.
fn build_cache_strategy(cfg: &ClariaConfig, model_id: &str) -> claria_bedrock::chat::CacheStrategy {
    if !cfg.prompt_caching_enabled {
        return claria_bedrock::chat::CacheStrategy::disabled();
    }
    let mut strategy = claria_bedrock::chat::CacheStrategy::enabled();
    strategy.model_supports_caching = model_supports_prompt_caching(model_id);
    strategy
}

/// True if the model_id (inference profile or bare foundation id) is a
/// Claude family known to honour Bedrock prompt-caching `cachePoint`
/// blocks at the time of writing.
fn model_supports_prompt_caching(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();
    // Claude 4 (Opus / Sonnet) and 3.5+ Haiku honour prompt caching with
    // 5-min TTL on Bedrock as of 2026-05-08.
    lower.contains("claude-opus-4")
        || lower.contains("claude-sonnet-4")
        || lower.contains("claude-3-5-haiku")
        || lower.contains("claude-haiku")
}

fn build_infra_system_prompt(plan_entries: &[PlanEntry]) -> String {
    let mut context = String::from("<infrastructure_context>\n");
    for entry in plan_entries {
        context.push_str(&format!(
            "<resource label=\"{}\" type=\"{}\" name=\"{}\">\n",
            entry.spec.label, entry.spec.resource_type, entry.spec.resource_name
        ));
        context.push_str(&format!(
            "  <description>{}</description>\n",
            entry.spec.description
        ));
        context.push_str(&format!(
            "  <desired_state>{}</desired_state>\n",
            serde_json::to_string_pretty(&entry.spec.desired).unwrap_or_default()
        ));
        if let Some(actual) = &entry.actual {
            context.push_str(&format!(
                "  <actual_state>{}</actual_state>\n",
                serde_json::to_string_pretty(actual).unwrap_or_default()
            ));
        }
        context.push_str(&format!("  <action>{:?}</action>\n", entry.action));
        context.push_str(&format!("  <cause>{:?}</cause>\n", entry.cause));
        if !entry.drift.is_empty() {
            context.push_str("  <drift>\n");
            for d in &entry.drift {
                context.push_str(&format!(
                    "    <field name=\"{}\" expected=\"{}\" actual=\"{}\" />\n",
                    d.field,
                    serde_json::to_string(&d.expected).unwrap_or_default(),
                    serde_json::to_string(&d.actual).unwrap_or_default()
                ));
            }
            context.push_str("  </drift>\n");
        }
        context.push_str("</resource>\n");
    }
    context.push_str("</infrastructure_context>");

    format!(
        r#"You are Claria's infrastructure assistant. Claria is a desktop application for
healthcare clinicians that runs entirely in the user's own AWS account — there is
no middleman, no third-party server, and no data leaves the user's control.

## How Claria works
- The clinician installs the Claria desktop app on their computer.
- Claria provisions and manages AWS resources in the clinician's own AWS account.
- All client records, chat history, and files are stored in a private S3 bucket.
- The clinician's AWS credentials never leave their machine.

## AWS services used
- **S3**: Stores all client data — records, files, chat history, and the search index.
  Configured with versioning, server-side encryption (AES-256), and a bucket policy
  that blocks public access.
- **CloudTrail**: Audit logging — every API call to the S3 bucket is recorded.
- **Bedrock**: AI model access for chat conversations and report generation.
  Claria uses cross-region inference profiles for model availability.
- **Transcribe**: Audio transcription for voice memos.
- **IAM**: A dedicated least-privilege IAM user with a scoped policy that grants
  only the permissions Claria needs. The policy is managed by Claria and kept in sync.

## HIPAA technical safeguards
- **Encryption at rest**: S3 server-side encryption (AES-256) for all stored data.
- **Encryption in transit**: All AWS API calls use TLS.
- **Access control**: Dedicated IAM user with least-privilege policy.
- **Audit logging**: CloudTrail records all S3 data events.
- **Versioning**: S3 versioning protects against accidental deletion.
- **No public access**: Bucket policy and public access block prevent exposure.
- **BAA**: AWS Business Associate Agreement covers HIPAA-eligible services.

## Instructions
Answer questions about the infrastructure using the context below. Be specific —
reference actual resource names, their current state, and their purpose. If the
user asks whether something is configured correctly, compare the desired state to
the actual state and note any drift. Be concise and direct.

{context}"#
    )
}

/// List named chat sessions for the Record screen history folder.
#[tauri::command]
#[specta::specta]
pub async fn list_chat_histories(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<Vec<ChatHistorySummary>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let rows = chat_history_rows(&s3, &bucket, client_id).await?;
    let mut summaries: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(index, (history, size))| chat_history_summary(history, *size, index + 1))
        .collect();
    summaries.reverse();
    Ok(summaries)
}

/// Rename a persisted chat session without changing its stable UUID key.
#[tauri::command]
#[specta::specta]
pub async fn rename_chat_history(
    state: State<'_, DesktopState>,
    client_id: String,
    chat_id: String,
    name: String,
) -> Result<ChatHistoryDetail, String> {
    let name = normalized_chat_name(&name)?;
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);
    let client_id = client_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let chat_id = chat_id
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let (mut history, etag) = stored_chat_history(&s3, &bucket, client_id, chat_id).await?;
    history.name = name;
    history.updated_at = jiff::Timestamp::now();
    let body = serde_json::to_vec_pretty(&history).map_err(|error| error.to_string())?;
    claria_storage::objects::put_object_if_match(
        &s3,
        &bucket,
        &claria_core::s3_keys::chat_history(client_id, chat_id),
        body,
        Some("application/json"),
        &etag,
    )
    .await
    .map_err(|error| match error {
        claria_storage::error::StorageError::PreconditionFailed { .. } => {
            "The chat changed on another computer. Reload it before renaming.".to_string()
        }
        other => other.to_string(),
    })?;

    record_audit(
        &sdk_config,
        &cfg,
        claria_audit::events::AuditEvent::new(
            "chat_history_renamed",
            "client",
            client_id.to_string(),
            cfg.account_id.clone(),
        )
        .with_details(serde_json::json!({ "chat_id": chat_id.to_string() })),
    )
    .await;

    Ok(chat_history_detail(history, "Chat (1)".to_string()))
}

/// Load a chat history session from S3.
///
/// Returns the full conversation with model ID so the frontend can
/// resume the session in the Chat widget.
#[tauri::command]
#[specta::specta]
pub async fn load_chat_history(
    state: State<'_, DesktopState>,
    client_id: String,
    chat_id: String,
) -> Result<ChatHistoryDetail, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let client_uuid: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let chat_uuid: uuid::Uuid = chat_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let (history, _) = stored_chat_history(&s3, &bucket, client_uuid, chat_uuid).await?;
    let fallback_name = if history.name.trim().is_empty() {
        let rows = chat_history_rows(&s3, &bucket, client_uuid).await?;
        rows.iter()
            .position(|(candidate, _)| candidate.id == chat_uuid)
            .map_or_else(
                || "Chat (1)".to_string(),
                |index| format!("Chat ({})", index + 1),
            )
    } else {
        history.name.clone()
    };

    Ok(chat_history_detail(history, fallback_name))
}

/// Accept the Marketplace agreement for a Bedrock foundation model.
///
/// Called when a model requires an agreement before it can be used.
/// The frontend can detect this from the error message and offer
/// a one-click accept flow.
#[tauri::command]
#[specta::specta]
pub async fn accept_model_agreement(
    state: State<'_, DesktopState>,
    model_id: String,
) -> Result<(), String> {
    let (_cfg, sdk_config) = load_sdk_config(&state).await?;

    claria_bedrock::chat::accept_model_agreement(&sdk_config, &model_id)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Prompt commands — editable prompts stored under claria-prompts/ in S3
// ---------------------------------------------------------------------------

/// Get the current content of a named prompt.
///
/// Returns the custom prompt from S3 if one exists, otherwise returns the
/// built-in default. Valid prompt names: `"system-prompt"`, `"pdf-extraction"`.
#[tauri::command]
#[specta::specta]
pub async fn get_prompt(
    state: State<'_, DesktopState>,
    prompt_name: String,
) -> Result<String, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    load_prompt(&s3, &bucket, &prompt_name).await
}

/// Save a named prompt to S3.
///
/// Overwrites any previously saved version. The new content takes effect on
/// the next operation that uses this prompt.
#[tauri::command]
#[specta::specta]
pub async fn save_prompt(
    state: State<'_, DesktopState>,
    prompt_name: String,
    content: String,
) -> Result<(), String> {
    let (key, _, _) = resolve_prompt(&prompt_name)?;

    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    claria_storage::objects::put_object(
        &s3,
        &bucket,
        key,
        content.into_bytes(),
        Some("text/markdown"),
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Delete a named prompt from S3, reverting to the built-in default.
#[tauri::command]
#[specta::specta]
pub async fn delete_prompt(
    state: State<'_, DesktopState>,
    prompt_name: String,
) -> Result<(), String> {
    let (key, _, _) = resolve_prompt(&prompt_name)?;

    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    claria_storage::objects::delete_object(&s3, &bucket, key)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Prompt version history commands
// ---------------------------------------------------------------------------

/// List all versions of a named prompt stored in S3.
#[tauri::command]
#[specta::specta]
pub async fn list_prompt_versions(
    state: State<'_, DesktopState>,
    prompt_name: String,
) -> Result<Vec<FileVersion>, String> {
    let (key, _, _) = resolve_prompt(&prompt_name)?;

    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let versions = claria_storage::objects::list_object_versions(&s3, &bucket, key)
        .await
        .map_err(|e| e.to_string())?;

    Ok(versions
        .into_iter()
        .filter(|v| !v.is_delete_marker)
        .map(|v| FileVersion {
            version_id: v.version_id,
            size: v.size as i32,
            last_modified: v.last_modified,
            is_latest: v.is_latest,
        })
        .collect())
}

/// Get the text content of a specific version of a named prompt.
#[tauri::command]
#[specta::specta]
pub async fn get_prompt_version(
    state: State<'_, DesktopState>,
    prompt_name: String,
    version_id: String,
) -> Result<String, String> {
    let (key, _, _) = resolve_prompt(&prompt_name)?;

    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let output = claria_storage::objects::get_object_version(&s3, &bucket, key, &version_id)
        .await
        .map_err(|e| e.to_string())?;

    String::from_utf8(output.body).map_err(|e| e.to_string())
}

/// Restore a previous version of a named prompt by writing it as the new current version.
#[tauri::command]
#[specta::specta]
pub async fn restore_prompt_version(
    state: State<'_, DesktopState>,
    prompt_name: String,
    version_id: String,
) -> Result<(), String> {
    let (key, _, _) = resolve_prompt(&prompt_name)?;

    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let output = claria_storage::objects::get_object_version(&s3, &bucket, key, &version_id)
        .await
        .map_err(|e| e.to_string())?;

    claria_storage::objects::put_object(&s3, &bucket, key, output.body, Some("text/markdown"))
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!(prompt_name, version_id, "prompt version restored");

    Ok(())
}

// ---------------------------------------------------------------------------
// Version history commands — S3 versioning surface
// ---------------------------------------------------------------------------

/// A single version of a file in a client's record.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FileVersion {
    pub version_id: String,
    pub size: i32,
    pub last_modified: Option<String>,
    pub is_latest: bool,
}

/// A file that has been deleted (has a delete marker as the latest version).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct DeletedFile {
    pub filename: String,
    pub deleted_at: Option<String>,
    pub version_id: String,
}

/// A client that has been deleted (has a delete marker on the client JSON).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct DeletedClient {
    pub id: String,
    pub name: String,
    pub deleted_at: Option<String>,
    pub version_id: String,
}

/// List all versions of a specific file in a client's record.
#[tauri::command]
#[specta::specta]
pub async fn list_file_versions(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
) -> Result<Vec<FileVersion>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let key = claria_core::s3_keys::client_record_file(id, &filename);

    let versions = claria_storage::objects::list_object_versions(&s3, &bucket, &key)
        .await
        .map_err(|e| e.to_string())?;

    Ok(versions
        .into_iter()
        .filter(|v| !v.is_delete_marker)
        .map(|v| FileVersion {
            version_id: v.version_id,
            size: v.size as i32,
            last_modified: v.last_modified,
            is_latest: v.is_latest,
        })
        .collect())
}

/// Get the text content of a specific version of a file.
#[tauri::command]
#[specta::specta]
pub async fn get_file_version_text(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    version_id: String,
) -> Result<String, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let key = claria_core::s3_keys::client_record_file(id, &filename);

    let output = claria_storage::objects::get_object_version(&s3, &bucket, &key, &version_id)
        .await
        .map_err(|e| e.to_string())?;

    String::from_utf8(output.body).map_err(|e| e.to_string())
}

/// Restore a previous version of a file by copying its content to a new PUT.
#[tauri::command]
#[specta::specta]
pub async fn restore_file_version(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    version_id: String,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let key = claria_core::s3_keys::client_record_file(id, &filename);

    // Fetch the old version's content.
    let output = claria_storage::objects::get_object_version(&s3, &bucket, &key, &version_id)
        .await
        .map_err(|e| e.to_string())?;

    // Write it back as the current version.
    claria_storage::objects::put_object(
        &s3,
        &bucket,
        &key,
        output.body,
        output.content_type.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;

    tracing::info!(client_id = %id, filename, version_id, "file version restored");

    Ok(())
}

/// List deleted files in a client's record (files with a delete marker).
#[tauri::command]
#[specta::specta]
pub async fn list_deleted_files(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<Vec<DeletedFile>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let prefix = claria_core::s3_keys::client_records_prefix(id);

    let deleted = claria_storage::objects::list_deleted_objects(&s3, &bucket, &prefix)
        .await
        .map_err(|e| e.to_string())?;

    // Collect all deleted keys so we can hide sidecar `.text` files.
    let entries: Vec<_> = deleted
        .iter()
        .filter_map(|d| {
            let filename = d.key.strip_prefix(&prefix)?;
            if filename.is_empty() {
                return None;
            }
            Some(filename.to_string())
        })
        .collect();
    let all_deleted: std::collections::HashSet<&str> = entries.iter().map(|s| s.as_str()).collect();

    Ok(deleted
        .into_iter()
        .filter_map(|d| {
            let filename = d.key.strip_prefix(&prefix)?.to_string();
            if filename.is_empty() {
                return None;
            }
            // Hide sidecar files: keys ending in `.text` where the base file
            // also has a delete marker (same logic as list_record_files).
            if let Some(base) = filename.strip_suffix(".text")
                && all_deleted.contains(base)
            {
                return None;
            }
            Some(DeletedFile {
                filename,
                deleted_at: d.last_modified,
                version_id: d.version_id,
            })
        })
        .collect())
}

/// Restore a deleted file by re-putting the most recent real version as a new version.
///
/// This preserves the full version history (including the delete marker) for
/// HIPAA audit-trail compliance, instead of removing the delete marker.
#[tauri::command]
#[specta::specta]
pub async fn restore_deleted_file(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    version_id: String,
) -> Result<(), String> {
    let _ = version_id; // kept for API compatibility; we find the latest real version ourselves
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let key = claria_core::s3_keys::client_record_file(id, &filename);

    // Find the most recent non-delete-marker version.
    let versions = claria_storage::objects::list_object_versions(&s3, &bucket, &key)
        .await
        .map_err(|e| e.to_string())?;
    let real = versions
        .iter()
        .find(|v| !v.is_delete_marker)
        .ok_or_else(|| format!("no restorable version found for {key}"))?;

    // Fetch that version's content and write it back as a new current version.
    let output = claria_storage::objects::get_object_version(&s3, &bucket, &key, &real.version_id)
            .await
            .map_err(|e| e.to_string())?;

    claria_storage::objects::put_object(
        &s3,
        &bucket,
        &key,
        output.body,
        output.content_type.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;

    tracing::info!(client_id = %id, filename, "deleted file restored");

    Ok(())
}

/// List deleted clients (client JSON files with a delete marker).
#[tauri::command]
#[specta::specta]
pub async fn list_deleted_clients(
    state: State<'_, DesktopState>,
) -> Result<Vec<DeletedClient>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let deleted = claria_storage::objects::list_deleted_objects(
        &s3,
        &bucket,
        claria_core::s3_keys::CLIENTS_PREFIX,
    )
    .await
    .map_err(|e| e.to_string())?;

    // Each deleted client costs two round-trips (ListObjectVersions, then
    // GetObject on the surviving version), which ran serially. `buffered`
    // caps in-flight requests and yields results in input order, so the
    // returned listing keeps the order `list_deleted_objects` produced.
    // The futures are collected up front so the stream borrows them with a
    // concrete lifetime — mapping references straight into `buffered` trips
    // a higher-ranked-lifetime error.
    let lookups: Vec<_> = deleted
        .iter()
        .map(|d| deleted_client_name(&s3, &bucket, &d.key))
        .collect();
    let names: Vec<Result<String, String>> = futures::stream::iter(lookups)
        .buffered(claria_desktop::records::S3_FETCH_CONCURRENCY)
        .collect()
        .await;

    let mut clients = Vec::with_capacity(deleted.len());
    for (d, name) in deleted.iter().zip(names) {
        // Extract the UUID from the key (e.g. "clients/abc-123.json" → "abc-123")
        let id = d
            .key
            .strip_prefix(claria_core::s3_keys::CLIENTS_PREFIX)
            .and_then(|s| s.strip_suffix(".json"))
            .unwrap_or(&d.key)
            .to_string();

        clients.push(DeletedClient {
            id,
            name: name?,
            deleted_at: d.last_modified.clone(),
            version_id: d.version_id.clone(),
        });
    }

    Ok(clients)
}

/// Name shown for a deleted client whose JSON can't be read back.
const UNKNOWN_CLIENT_NAME: &str = "Unknown";

/// Resolve a deleted client's display name from its most recent
/// non-delete-marker version.
///
/// Only the version listing is fatal. A missing, unreadable, or unparseable
/// body degrades to [`UNKNOWN_CLIENT_NAME`] so one corrupt object doesn't
/// blank the whole deleted-items list.
async fn deleted_client_name(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<String, String> {
    let versions = claria_storage::objects::list_object_versions(s3, bucket, key)
        .await
        .map_err(|e| e.to_string())?;

    let Some(version) = versions.iter().find(|v| !v.is_delete_marker) else {
        tracing::warn!(
            key,
            version_count = versions.len(),
            "no non-delete-marker version found for deleted client"
        );
        return Ok(UNKNOWN_CLIENT_NAME.to_string());
    };

    if version.version_id.is_empty() {
        tracing::warn!(
            key,
            "deleted client has empty version_id (pre-versioning object)"
        );
        return Ok(UNKNOWN_CLIENT_NAME.to_string());
    }

    let version_id = &version.version_id;
    let output = match claria_storage::objects::get_object_version(s3, bucket, key, version_id)
        .await
    {
        Ok(output) => output,
        Err(e) => {
            tracing::warn!(key, version_id, error = %e, "failed to fetch deleted client version");
            return Ok(UNKNOWN_CLIENT_NAME.to_string());
        }
    };

    match serde_json::from_slice::<claria_core::models::client::Client>(&output.body) {
        Ok(client) => Ok(client.name),
        Err(e) => {
            tracing::warn!(key, version_id, error = %e, "failed to deserialize deleted client JSON");
            Ok(UNKNOWN_CLIENT_NAME.to_string())
        }
    }
}

/// Restore a deleted client by re-putting the most recent real version as a new version.
///
/// This preserves the full version history (including the delete marker) for
/// HIPAA audit-trail compliance, instead of removing the delete marker.
#[tauri::command]
#[specta::specta]
pub async fn restore_client(
    state: State<'_, DesktopState>,
    client_id: String,
    version_id: String,
) -> Result<(), String> {
    let _ = version_id; // kept for API compatibility; we find the latest real version ourselves
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let outcome = claria_client_lifecycle::restore_client(&s3, &bucket, id)
        .await
        .map_err(|error| error.to_string())?;

    tracing::info!(
        client_id = %id,
        report_restored = outcome.report_restored,
        "deleted client restored"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Update check
// ---------------------------------------------------------------------------

/// Result of checking for a newer release on GitHub.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct UpdateCheck {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_url: String,
}

/// Check whether a newer release exists on GitHub.
///
/// Hits the GitHub releases API and compares `tag_name` against the compiled-in
/// version. On any failure (network, parse) returns `update_available: false` so
/// the UI never errors out.
#[tauri::command]
#[specta::specta]
pub async fn check_for_updates() -> Result<UpdateCheck, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();

    let result: Result<UpdateCheck, String> = tokio::task::spawn_blocking({
        let current = current.clone();
        move || {
            let agent = ureq::Agent::new_with_config(
                ureq::config::Config::builder()
                    .timeout_global(Some(std::time::Duration::from_secs(5)))
                    .build(),
            );
            let resp = agent
                .get("https://api.github.com/repos/claria-ai/claria/releases/latest")
                .header("User-Agent", "claria-desktop")
                .header("Accept", "application/vnd.github+json")
                .call()
                .map_err(|e| format!("{e}"))?;

            let body_str = resp
                .into_body()
                .read_to_string()
                .map_err(|e| e.to_string())?;
            let body: serde_json::Value =
                serde_json::from_str(&body_str).map_err(|e| e.to_string())?;

            let tag = body["tag_name"].as_str().ok_or("missing tag_name")?;
            let latest = tag.strip_prefix('v').unwrap_or(tag).to_string();
            let release_url = body["html_url"]
                .as_str()
                .unwrap_or("https://github.com/claria-ai/claria/releases")
                .to_string();

            let update_available = claria_desktop::update::update_available(&current, &latest);

            Ok(UpdateCheck {
                current_version: current,
                latest_version: latest,
                update_available,
                release_url,
            })
        }
    })
    .await
    .map_err(|e| format!("update check task failed: {e}"))?;

    // On error, return a safe default instead of propagating.
    Ok(result.unwrap_or(UpdateCheck {
        current_version: current.clone(),
        latest_version: current,
        update_available: false,
        release_url: "https://github.com/claria-ai/claria/releases".to_string(),
    }))
}

// ---------------------------------------------------------------------------
// Cost Explorer commands
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn get_cost_and_usage(
    state: State<'_, DesktopState>,
    start_date: String,
    end_date: String,
    granularity: claria_billing::CostGranularity,
    group_by_service: bool,
) -> Result<claria_billing::CostAndUsageResult, String> {
    let (_cfg, sdk_config) = load_sdk_config(&state).await?;
    let query = claria_billing::CostQuery {
        start_date,
        end_date,
        granularity,
        group_by_service,
    };
    claria_billing::get_cost_and_usage(&sdk_config, &query)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn probe_cost_explorer(state: State<'_, DesktopState>) -> Result<(), String> {
    let (_cfg, sdk_config) = load_sdk_config(&state).await?;
    claria_billing::probe_cost_explorer(&sdk_config)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn enable_cost_explorer(state: State<'_, DesktopState>) -> Result<(), String> {
    let (mut cfg, sdk_config) = load_sdk_config(&state).await?;
    cfg.cost_explorer_enabled = true;
    save_config_synced(&state, &sdk_config, cfg, "Cost Explorer setting").await
}

#[tauri::command]
#[specta::specta]
pub async fn set_hourly_cost_data(
    state: State<'_, DesktopState>,
    enabled: bool,
) -> Result<(), String> {
    let (mut cfg, sdk_config) = load_sdk_config(&state).await?;
    cfg.hourly_cost_data = enabled;
    save_config_synced(&state, &sdk_config, cfg, "hourly cost data setting").await
}

/// Look up `ModelPricing` for a Bedrock model_id. Returns `None` for
/// unknown models so the UI can hide pre-flight estimates rather than
/// show `$NaN`.
#[tauri::command]
#[specta::specta]
pub async fn lookup_model_pricing(
    model_id: String,
) -> Result<Option<claria_core::models::cost::ModelPricing>, String> {
    Ok(claria_billing::pricing::lookup(&model_id))
}

// ---------------------------------------------------------------------------
// Shell / URL helpers
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("URL must start with http:// or https://".into());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Token counting commands
// ---------------------------------------------------------------------------
//
// Neither command takes a model: the count always runs against the newest
// available Haiku, because not every chat model supports `CountTokens`.

#[tauri::command]
#[specta::specta]
pub async fn count_client_context_tokens(
    state: State<'_, DesktopState>,
    client_id: String,
    context_filenames: Vec<String>,
) -> Result<u32, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = claria_storage::client::from_config(&sdk_config);
    let bucket = bucket_name(&cfg);

    let system_prompt = load_prompt(&s3, &bucket, "system-prompt").await?;
    let all_files = load_record_context(&s3, &bucket, &client_id, &state.record_cache).await?;
    let files: Vec<_> = if context_filenames.is_empty() {
        all_files
    } else {
        let allowed: std::collections::HashSet<&str> =
            context_filenames.iter().map(|s| s.as_str()).collect();
        all_files
            .into_iter()
            .filter(|f| allowed.contains(f.filename.as_str()))
            .collect()
    };
    let context_block = claria_bedrock::context::build_context_block(&files);
    let full_prompt = if context_block.is_empty() {
        system_prompt
    } else {
        format!("{context_block}\n\n{system_prompt}")
    };

    claria_bedrock::chat::count_context_tokens(&sdk_config, &full_prompt)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn count_infra_context_tokens(
    state: State<'_, DesktopState>,
    plan_entries: Vec<PlanEntry>,
) -> Result<u32, String> {
    let (_cfg, sdk_config) = load_sdk_config(&state).await?;
    let system_prompt = build_infra_system_prompt(&plan_entries);

    claria_bedrock::chat::count_context_tokens(&sdk_config, &system_prompt)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Console log commands
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
pub fn get_console_logs(console: State<'_, ConsoleBuffer>) -> Vec<ConsoleEntry> {
    console.entries()
}

#[tauri::command]
#[specta::specta]
pub fn get_console_logs_text(console: State<'_, ConsoleBuffer>) -> String {
    console.to_text()
}

#[tauri::command]
#[specta::specta]
pub fn save_console_logs(console: State<'_, ConsoleBuffer>) -> Result<bool, String> {
    let text = console.to_text();
    let date = jiff::Timestamp::now().strftime("%Y-%m-%d").to_string();

    let path = rfd::FileDialog::new()
        .set_file_name(format!("claria-console-{date}.log"))
        .add_filter("Log files", &["log", "txt"])
        .save_file();

    match path {
        Some(p) => {
            std::fs::write(&p, text).map_err(|e| e.to_string())?;
            Ok(true)
        }
        None => Ok(false),
    }
}