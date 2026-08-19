//! Config lifecycle and synced-preferences commands.

use tauri::State;

use claria_storage::audit::actions;

use claria_desktop::config::{
    self, ClariaConfig, ConfigInfo, CredentialSource, DraftPipelinePreferences,
    ReportAuthoringPreferences, SyncedPreferences, TranscriptionPreferences,
};

use super::{
    CommandContext, CommandError, bucket_name, cached_aws, run,
    versions::{FileVersion, get_version_text_for_key, list_versions_for_key},
};
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
                Ok(Some((synced, _etag))) => {
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

/// Read `_state/preferences.json` from S3, with the object's ETag for
/// conditional writes. Returns `Ok(None)` when the object doesn't exist
/// (first launch, fresh provisioner); errors only on transport failure or
/// malformed JSON.
async fn read_cloud_preferences(
    s3: &aws_sdk_s3::Client,
    cfg: &ClariaConfig,
) -> Result<Option<(SyncedPreferences, Option<String>)>, CommandError> {
    let bucket = bucket_name(cfg);
    match claria_storage::objects::get_object(s3, &bucket, claria_core::s3_keys::PREFERENCES).await
    {
        Ok(output) => {
            let synced: SyncedPreferences = serde_json::from_slice(&output.body)?;
            synced.report_authoring.validate()?;
            Ok(Some((synced, output.etag)))
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

/// Named-field patch for the synced preferences. Absent fields are left
/// untouched, so a UI section (or a single-setting command) saves only what
/// it owns and can never roll back a sibling section's edit.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct PreferencesPatch {
    /// `Some(Some(id))` sets the preferred model, `Some(None)` clears it
    /// (only expressible in-process — over IPC use `set_preferred_model`),
    /// `None` leaves it unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub preferred_model_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub cost_explorer_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub hourly_cost_data: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub prompt_caching_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub transcription: Option<TranscriptionPreferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub report_authoring: Option<ReportAuthoringPreferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub model_tuning: Option<claria_desktop::config::ModelTuningPreferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub chat_streaming: Option<claria_desktop::config::ChatStreamMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub draft_pipeline: Option<DraftPipelinePreferences>,
}

impl PreferencesPatch {
    fn apply_to(&self, synced: &mut SyncedPreferences) {
        if let Some(preferred_model_id) = &self.preferred_model_id {
            synced.preferred_model_id = preferred_model_id.clone();
        }
        if let Some(cost_explorer_enabled) = self.cost_explorer_enabled {
            synced.cost_explorer_enabled = cost_explorer_enabled;
        }
        if let Some(hourly_cost_data) = self.hourly_cost_data {
            synced.hourly_cost_data = hourly_cost_data;
        }
        if let Some(prompt_caching_enabled) = self.prompt_caching_enabled {
            synced.prompt_caching_enabled = prompt_caching_enabled;
        }
        if let Some(transcription) = &self.transcription {
            synced.transcription = transcription.clone();
        }
        if let Some(report_authoring) = &self.report_authoring {
            synced.report_authoring = report_authoring.clone();
        }
        if let Some(model_tuning) = self.model_tuning {
            synced.model_tuning = model_tuning;
        }
        if let Some(chat_streaming) = self.chat_streaming {
            synced.chat_streaming = chat_streaming;
        }
        if let Some(draft_pipeline) = &self.draft_pipeline {
            synced.draft_pipeline = draft_pipeline.clone();
        }
    }
}

/// Conflict-retry budget for the preferences read-modify-write loop.
const PREFERENCES_RMW_ATTEMPTS: u32 = 3;

/// Apply a [`PreferencesPatch`] locally and to `_state/preferences.json`.
///
/// Every command that changes a [`SyncedPreferences`] field must go through
/// here. `load_config` overlays the S3 copy onto the local config on each
/// call, so a local-only write is silently reverted by the next read — the
/// setting appears to save, survives on disk, and still comes back stale.
///
/// The local write happens first so the edit is not lost when S3 is
/// unreachable; the cloud failure is returned so the UI can say so. The
/// cloud write is a read-modify-write against the current object under an
/// ETag precondition, so two sections (or machines) saving concurrently
/// merge instead of clobbering each other's fields.
pub(crate) async fn apply_preferences_patch(
    state: &State<'_, DesktopState>,
    s3: &aws_sdk_s3::Client,
    mut cfg: ClariaConfig,
    patch: &PreferencesPatch,
    what: &str,
) -> Result<ClariaConfig, CommandError> {
    if let Some(report_authoring) = &patch.report_authoring {
        report_authoring.validate()?;
    }
    if let Some(model_tuning) = &patch.model_tuning {
        model_tuning.validate()?;
    }

    // Persist locally first so we don't lose the user's edit if S3 is down.
    let mut local = SyncedPreferences::from_config(&cfg);
    patch.apply_to(&mut local);
    local.apply_to_config(&mut cfg);
    config::save_config(&cfg)?;

    let bucket = bucket_name(&cfg);
    let mut attempts = 0;
    let merged = loop {
        attempts += 1;
        let (mut synced, etag) = match read_cloud_preferences(s3, &cfg).await? {
            Some((synced, etag)) => (synced, etag),
            None => (SyncedPreferences::from_config(&cfg), None),
        };
        patch.apply_to(&mut synced);
        let body = serde_json::to_vec_pretty(&synced)?;
        let put = match etag.as_deref() {
            Some(etag) => {
                claria_storage::objects::put_object_if_match(
                    s3,
                    &bucket,
                    claria_core::s3_keys::PREFERENCES,
                    body,
                    Some("application/json"),
                    etag,
                )
                .await
            }
            None => {
                claria_storage::objects::put_object_if_none_match(
                    s3,
                    &bucket,
                    claria_core::s3_keys::PREFERENCES,
                    body,
                    Some("application/json"),
                )
                .await
            }
        };
        match put {
            Ok(_) => break synced,
            Err(claria_storage::error::StorageError::PreconditionFailed { .. })
                if attempts < PREFERENCES_RMW_ATTEMPTS =>
            {
                continue;
            }
            Err(e) => {
                // The local edit is already saved; make the in-memory copy
                // match it before reporting the partial save.
                let mut guard = state.config.lock().await;
                *guard = Some(cfg);
                return Err(CommandError::Msg(format!(
                    "{what} saved locally but cloud sync failed: {e}"
                )));
            }
        }
    };

    // The merge may have pulled in sibling fields another machine changed;
    // fold the authoritative cloud result back into the local config.
    merged.apply_to_config(&mut cfg);
    config::save_config(&cfg)?;

    let mut guard = state.config.lock().await;
    *guard = Some(cfg.clone());
    Ok(cfg)
}

/// Save one UI section's preference fields. Absent patch fields are left
/// untouched locally and in `_state/preferences.json`, so concurrent
/// sections can't clobber each other. Bubbles S3-write failures so the
/// frontend can show a partial-save warning.
#[tauri::command]
#[specta::specta]
pub async fn save_preferences_patch(
    state: State<'_, DesktopState>,
    patch: PreferencesPatch,
) -> Result<ConfigInfo, String> {
    run("save_preferences_patch", async {
        let ctx = CommandContext::new(&state).await?;
        let cfg = apply_preferences_patch(&state, &ctx.s3, ctx.cfg, &patch, "preferences").await?;
        Ok(config::config_info(&cfg))
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
        if let Some((synced, _etag)) = read_cloud_preferences(&ctx.s3, &cfg).await? {
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
            model_tuning: Default::default(),
            chat_streaming: Default::default(),
            draft_pipeline: Default::default(),
            security: Default::default(),
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
        let patch = PreferencesPatch {
            preferred_model_id: Some(model_id),
            ..Default::default()
        };
        apply_preferences_patch(&state, &ctx.s3, ctx.cfg, &patch, "preferred model").await?;
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Preferences file: export, import, and version history
// ---------------------------------------------------------------------------

/// Ceiling for an imported preferences file. The real file is a few KiB, so
/// anything past this is a mis-picked file, not a preferences export.
const MAX_PREFERENCES_IMPORT_BYTES: u64 = 1024 * 1024;

/// Parse a user-supplied preferences JSON body. Rejects a file written by a
/// newer Claria rather than silently dropping its unknown fields, which a
/// rewrite would otherwise erase from the synced copy. Field validation
/// happens in [`apply_preferences_patch`], which every accepted import and
/// restore goes through.
fn parse_preferences_file(bytes: &[u8]) -> Result<SyncedPreferences, CommandError> {
    let synced: SyncedPreferences = serde_json::from_slice(bytes).map_err(|e| {
        CommandError::Msg(format!(
            "The selected file is not a Claria preferences export: {e}"
        ))
    })?;
    if synced.preferences_version > config::PREFERENCES_VERSION {
        return Err(CommandError::Msg(
            "This preferences file was written by a newer Claria. Update Claria, then try again."
                .to_string(),
        ));
    }
    Ok(synced)
}

/// A patch that sets every synced field — a whole-file replace expressed
/// through the same merge path section saves use.
fn full_preferences_patch(synced: &SyncedPreferences) -> PreferencesPatch {
    PreferencesPatch {
        preferred_model_id: Some(synced.preferred_model_id.clone()),
        cost_explorer_enabled: Some(synced.cost_explorer_enabled),
        hourly_cost_data: Some(synced.hourly_cost_data),
        prompt_caching_enabled: Some(synced.prompt_caching_enabled),
        transcription: Some(synced.transcription.clone()),
        report_authoring: Some(synced.report_authoring.clone()),
        model_tuning: Some(synced.model_tuning),
        chat_streaming: Some(synced.chat_streaming),
        draft_pipeline: Some(synced.draft_pipeline.clone()),
    }
}

/// Save the S3-stored preferences file to a user-selected local path, verbatim
/// so a support reader sees exactly what the app reads. Falls back to a
/// canonical serialization of the local values when the cloud copy doesn't
/// exist yet. Returns `false` when the dialog is cancelled.
#[tauri::command]
#[specta::specta]
pub async fn export_preferences(state: State<'_, DesktopState>) -> Result<bool, String> {
    run("export_preferences", async {
        let ctx = CommandContext::new(&state).await?;
        let bytes = match claria_storage::objects::get_object(
            &ctx.s3,
            &ctx.bucket,
            claria_core::s3_keys::PREFERENCES,
        )
        .await
        {
            Ok(output) => output.body,
            Err(claria_storage::error::StorageError::NotFound { .. }) => {
                serde_json::to_vec_pretty(&SyncedPreferences::from_config(&ctx.cfg))?
            }
            Err(e) => return Err(e.into()),
        };

        let date = jiff::Timestamp::now().strftime("%Y-%m-%d").to_string();
        // Async dialog: on macOS the sync dialog cannot follow async S3 work
        // (see the DOCX export), returning as cancelled repeatedly.
        let selected = rfd::AsyncFileDialog::new()
            .set_title("Export Claria preferences")
            .set_file_name(format!("claria-preferences-{date}.json"))
            .add_filter("JSON", &["json"])
            .save_file()
            .await;
        let Some(selected) = selected else {
            return Ok(false);
        };
        claria_desktop::local_export::write_private_atomic(selected.path(), &bytes)?;
        Ok(true)
    })
    .await
}

/// Replace the synced preferences with a user-selected export. The previous
/// values stay one entry back in the file's S3 version history. Returns
/// `None` when the dialog is cancelled.
#[tauri::command]
#[specta::specta]
pub async fn import_preferences(
    state: State<'_, DesktopState>,
) -> Result<Option<ConfigInfo>, String> {
    run("import_preferences", async {
        let selected = rfd::AsyncFileDialog::new()
            .set_title("Import Claria preferences")
            .add_filter("JSON", &["json"])
            .pick_file()
            .await;
        let Some(selected) = selected else {
            return Ok(None);
        };
        let metadata = tokio::fs::metadata(selected.path()).await.map_err(|_| {
            CommandError::Msg("Claria could not inspect the selected file.".to_string())
        })?;
        if metadata.len() > MAX_PREFERENCES_IMPORT_BYTES {
            return Err(CommandError::Msg(
                "The selected file is too large to be a Claria preferences export.".to_string(),
            ));
        }
        let bytes = tokio::fs::read(selected.path()).await.map_err(|_| {
            CommandError::Msg("Claria could not read the selected file.".to_string())
        })?;
        let synced = parse_preferences_file(&bytes)?;

        let ctx = CommandContext::new(&state).await?;
        let patch = full_preferences_patch(&synced);
        let cfg = apply_preferences_patch(
            &state,
            &ctx.s3,
            ctx.cfg.clone(),
            &patch,
            "imported preferences",
        )
        .await?;
        ctx.record_audit(ctx.audit_event(
            actions::PREFERENCES_IMPORT,
            "preferences",
            claria_core::s3_keys::PREFERENCES,
        ))
        .await;
        Ok(Some(config::config_info(&cfg)))
    })
    .await
}

/// List all versions of the synced preferences file.
#[tauri::command]
#[specta::specta]
pub async fn list_preferences_versions(
    state: State<'_, DesktopState>,
) -> Result<Vec<FileVersion>, String> {
    run("list_preferences_versions", async {
        let ctx = CommandContext::new(&state).await?;
        list_versions_for_key(&ctx.s3, &ctx.bucket, claria_core::s3_keys::PREFERENCES).await
    })
    .await
}

/// Get the text of one version of the synced preferences file.
#[tauri::command]
#[specta::specta]
pub async fn get_preferences_version(
    state: State<'_, DesktopState>,
    version_id: String,
) -> Result<String, String> {
    run("get_preferences_version", async {
        let ctx = CommandContext::new(&state).await?;
        get_version_text_for_key(
            &ctx.s3,
            &ctx.bucket,
            claria_core::s3_keys::PREFERENCES,
            &version_id,
        )
        .await
    })
    .await
}

/// Restore a previous version of the synced preferences file. The version's
/// content is parsed and validated first, then written through the normal
/// patch path so the local config follows and the overwritten values remain
/// in version history.
#[tauri::command]
#[specta::specta]
pub async fn restore_preferences_version(
    state: State<'_, DesktopState>,
    version_id: String,
) -> Result<ConfigInfo, String> {
    run("restore_preferences_version", async {
        let ctx = CommandContext::new(&state).await?;
        let text = get_version_text_for_key(
            &ctx.s3,
            &ctx.bucket,
            claria_core::s3_keys::PREFERENCES,
            &version_id,
        )
        .await?;
        let synced = parse_preferences_file(text.as_bytes())?;
        let patch = full_preferences_patch(&synced);
        let cfg = apply_preferences_patch(
            &state,
            &ctx.s3,
            ctx.cfg.clone(),
            &patch,
            "restored preferences",
        )
        .await?;
        tracing::info!(version_id, "preferences version restored");
        ctx.record_audit(
            ctx.audit_event(
                actions::PREFERENCES_VERSION_RESTORE,
                "preferences",
                claria_core::s3_keys::PREFERENCES,
            )
            .with_details(serde_json::json!({ "version_id": version_id })),
        )
        .await;
        Ok(config::config_info(&cfg))
    })
    .await
}
