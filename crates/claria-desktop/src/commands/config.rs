//! Config lifecycle and synced-preferences commands.

use tauri::State;

use claria_desktop::config::{
    self, ClariaConfig, ConfigInfo, CredentialSource, ReportAuthoringPreferences,
    SyncedPreferences, TranscriptionPreferences,
};

use super::{CommandContext, CommandError, bucket_name, cached_aws, run};
use crate::state::DesktopState;

#[tauri::command]
#[specta::specta]
pub async fn has_config() -> Result<bool, String> {
    Ok(config::has_config())
}

#[tauri::command]
#[specta::specta]
pub async fn load_config(state: State<'_, DesktopState>) -> Result<ConfigInfo, String> {
    run("load_config", async {
        let mut cfg = config::load_config()?;

        // Backfill account_id for configs saved before this field existed.
        if cfg.account_id.is_empty() {
            let (sdk_config, _) = cached_aws(&state, &cfg).await;
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
            let (_sdk_config, s3) = cached_aws(&state, &cfg).await;
            match read_cloud_preferences(&s3, &cfg).await {
                Ok(Some(synced)) => {
                    synced.apply_to_config(&mut cfg);
                    tracing::debug!("applied synced preferences from S3");
                }
                Ok(None) => {
                    // First boot against this bucket: seed the file with the local
                    // values so every later read (here and on other machines)
                    // finds it.
                    let synced = SyncedPreferences::from_config(&cfg);
                    match write_cloud_preferences(&s3, &cfg, &synced).await {
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
    })
    .await
}

/// Read `_state/preferences.json` from S3. Returns `Ok(None)` when the object
/// doesn't exist (first launch, fresh provisioner); errors only on transport
/// failure or malformed JSON.
async fn read_cloud_preferences(
    s3: &aws_sdk_s3::Client,
    cfg: &ClariaConfig,
) -> Result<Option<SyncedPreferences>, CommandError> {
    let bucket = bucket_name(cfg);
    match claria_storage::objects::get_object(s3, &bucket, claria_core::s3_keys::PREFERENCES).await
    {
        Ok(output) => {
            let synced: SyncedPreferences = serde_json::from_slice(&output.body)?;
            synced.report_authoring.validate()?;
            Ok(Some(synced))
        }
        Err(claria_storage::error::StorageError::NotFound { .. }) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Write `_state/preferences.json` to S3.
async fn write_cloud_preferences(
    s3: &aws_sdk_s3::Client,
    cfg: &ClariaConfig,
    synced: &SyncedPreferences,
) -> Result<(), CommandError> {
    let bucket = bucket_name(cfg);
    let body = serde_json::to_vec_pretty(synced)?;
    claria_storage::objects::put_object(
        s3,
        &bucket,
        claria_core::s3_keys::PREFERENCES,
        body,
        Some("application/json"),
    )
    .await?;
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
pub(crate) async fn save_config_synced(
    state: &State<'_, DesktopState>,
    s3: &aws_sdk_s3::Client,
    cfg: ClariaConfig,
    what: &str,
) -> Result<(), CommandError> {
    config::save_config(&cfg)?;

    let synced = SyncedPreferences::from_config(&cfg);
    let cloud = write_cloud_preferences(s3, &cfg, &synced)
        .await
        .map_err(|e| CommandError::Msg(format!("{what} saved locally but cloud sync failed: {e}")));

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
    run("save_preferences", async {
        let ctx = CommandContext::new(&state).await?;
        let mut cfg = ctx.cfg;

        cfg.preferred_model_id = preferred_model_id;
        cfg.cost_explorer_enabled = cost_explorer_enabled;
        cfg.hourly_cost_data = hourly_cost_data;
        cfg.prompt_caching_enabled = prompt_caching_enabled;
        cfg.transcription = transcription;
        report_authoring.validate()?;
        cfg.report_authoring = report_authoring;

        // Persist locally first so we don't lose the user's edit if S3 is down.
        config::save_config(&cfg)?;

        let synced = SyncedPreferences::from_config(&cfg);
        write_cloud_preferences(&ctx.s3, &cfg, &synced)
            .await
            .map_err(|e| {
                CommandError::Msg(format!(
                    "preferences saved locally but cloud sync failed: {e}"
                ))
            })?;

        let info = config::config_info(&cfg);
        let mut guard = state.config.lock().await;
        *guard = Some(cfg);
        Ok(info)
    })
    .await
}

/// Re-fetch synced preferences from S3 and overlay onto the in-memory config.
/// Used by the Preferences page on entry so users on the editing machine see
/// the latest cloud state without an app restart.
#[tauri::command]
#[specta::specta]
pub async fn fetch_cloud_preferences(state: State<'_, DesktopState>) -> Result<ConfigInfo, String> {
    run("fetch_cloud_preferences", async {
        let ctx = CommandContext::new(&state).await?;
        let mut cfg = ctx.cfg;
        if let Some(synced) = read_cloud_preferences(&ctx.s3, &cfg).await? {
            synced.apply_to_config(&mut cfg);
            config::save_config(&cfg)?;
        }
        let info = config::config_info(&cfg);
        let mut guard = state.config.lock().await;
        *guard = Some(cfg);
        Ok(info)
    })
    .await
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
    run("save_config", async {
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

        config::save_config(&cfg)?;

        let mut guard = state.config.lock().await;
        *guard = Some(cfg);

        Ok(())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_config(state: State<'_, DesktopState>) -> Result<(), String> {
    run("delete_config", async {
        config::delete_config()?;

        let mut guard = state.config.lock().await;
        *guard = None;

        Ok(())
    })
    .await
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
    run("set_preferred_model", async {
        let ctx = CommandContext::new(&state).await?;
        let mut cfg = ctx.cfg;
        cfg.preferred_model_id = model_id;
        save_config_synced(&state, &ctx.s3, cfg, "preferred model").await
    })
    .await
}
