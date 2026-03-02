use serde::{Deserialize, Serialize};
use tauri::State;

use claria_desktop::config::{self, ClariaConfig, ConfigInfo, CredentialSource};
use claria_provisioner::account_setup::{
    AccessKeyInfo, AssumeRoleResult, BootstrapResult, CredentialAssessment, CredentialClass,
    StepStatus,
};
use claria_provisioner::PlanEntry;

use crate::state::DesktopState;

// ---------------------------------------------------------------------------
// Client + Chat types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ClientSummary {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

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

/// Response from a chat message, including the persisted chat session ID.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ChatResponse {
    pub chat_id: String,
    pub content: String,
}

/// Detail of a persisted chat session, returned when resuming a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ChatHistoryDetail {
    pub chat_id: String,
    pub model_id: String,
    pub messages: Vec<ChatMessage>,
    pub created_at: String,
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
pub async fn load_config(
    state: State<'_, DesktopState>,
) -> Result<ConfigInfo, String> {
    let mut cfg = config::load_config().map_err(|e| e.to_string())?;

    // Backfill account_id for configs saved before this field existed.
    if cfg.account_id.is_empty() {
        let sdk_config =
            claria_desktop::aws::build_aws_config(&cfg.region, &cfg.credentials).await;
        let sts = aws_sdk_sts::Client::new(&sdk_config);
        if let Ok(identity) = sts.get_caller_identity().send().await
            && let Some(account_id) = identity.account()
        {
            cfg.account_id = account_id.to_string();
            // Best-effort re-save so next load doesn't need STS again.
            let _ = config::save_config(&cfg);
        }
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
    };

    config::save_config(&cfg).map_err(|e| e.to_string())?;

    let mut guard = state.config.lock().await;
    *guard = Some(cfg);

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_config(
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    config::delete_config().map_err(|e| e.to_string())?;

    let mut guard = state.config.lock().await;
    *guard = None;

    Ok(())
}

/// Set the clinician's preferred chat model.
///
/// Loads the current config, updates `preferred_model_id`, and saves. Pass
/// `None` to clear the preference (fall back to the first available model).
#[tauri::command]
#[specta::specta]
pub async fn set_preferred_model(
    state: State<'_, DesktopState>,
    model_id: Option<String>,
) -> Result<(), String> {
    let mut cfg = config::load_config().map_err(|e| e.to_string())?;
    cfg.preferred_model_id = model_id;
    config::save_config(&cfg).map_err(|e| e.to_string())?;

    let mut guard = state.config.lock().await;
    *guard = Some(cfg);

    Ok(())
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
    let sdk_config =
        claria_desktop::aws::build_aws_config(&region, &credentials).await;
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
    let sdk_config =
        claria_desktop::aws::build_aws_config(&region, &credentials).await;

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
    let sdk_config =
        claria_desktop::aws::build_aws_config(&region, &credentials).await;
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
    let sdk_config =
        claria_desktop::aws::build_aws_config(&region, &credentials).await;
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
                            parts.push(format!("Accepted {} model(s)", summary.newly_accepted.len()));
                        }
                        if !summary.failed.is_empty() {
                            parts.push(format!("{} failed", summary.failed.len()));
                        }
                        parts.join(", ")
                    };

                    let step = result.steps.iter_mut().rfind(|s| s.name == "accept_model_agreements");
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
                    let step = result.steps.iter_mut().rfind(|s| s.name == "accept_model_agreements");
                    if let Some(s) = step {
                        s.status = StepStatus::Failed;
                        s.detail = Some(format!("Non-fatal: {e}. You can accept model agreements later from the chat screen."));
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
) -> Result<(), String> {
    let (cfg, _) = load_sdk_config(&state).await?;

    let elevated_config = claria_desktop::aws::build_aws_config(
        &cfg.region,
        &CredentialSource::Inline {
            access_key_id,
            secret_access_key,
            session_token: None,
        },
    )
    .await;

    claria_provisioner::update_iam_policy(
        &elevated_config,
        &cfg.system_name,
        &cfg.account_id,
    )
    .await
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Provisioner commands — scan, plan, provision, destroy
// ---------------------------------------------------------------------------

/// Helper: load the saved config and build an SDK config from it.
///
/// If the in-memory state is empty, attempts to load from disk first.
/// Returns `(ClariaConfig, SdkConfig)`. Errors if no config is saved yet.
async fn load_sdk_config(
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

    let sdk_config =
        claria_desktop::aws::build_aws_config(&cfg.region, &cfg.credentials).await;
    Ok((cfg, sdk_config))
}

/// Scan all resources and return an annotated plan.
///
/// This is always the first call — both onboarding and dashboard use it.
/// The plan is a flat `Vec<PlanEntry>`, each carrying the full spec plus
/// action/cause/drift so the frontend has everything it needs.
#[tauri::command]
#[specta::specta]
pub async fn plan(
    state: State<'_, DesktopState>,
) -> Result<Vec<PlanEntry>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let manifest = claria_provisioner::build_manifest(
        &cfg.account_id,
        &cfg.system_name,
        &cfg.region,
    );
    let syncers = claria_provisioner::build_syncers(&sdk_config, &manifest);
    let persistence = claria_provisioner::build_persistence(
        &sdk_config,
        &cfg.system_name,
        &cfg.account_id,
    )
    .map_err(|e| e.to_string())?;
    let prov_state = persistence.load().await.map_err(|e| e.to_string())?;

    claria_provisioner::plan(&syncers, &prov_state)
        .await
        .map_err(|e| e.to_string())
}

/// Execute all actionable entries in the plan.
///
/// Returns the updated plan (all entries should now be Ok).
#[tauri::command]
#[specta::specta]
pub async fn apply(
    state: State<'_, DesktopState>,
) -> Result<Vec<PlanEntry>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let manifest = claria_provisioner::build_manifest(
        &cfg.account_id,
        &cfg.system_name,
        &cfg.region,
    );
    let syncers = claria_provisioner::build_syncers(&sdk_config, &manifest);
    let persistence = claria_provisioner::build_persistence(
        &sdk_config,
        &cfg.system_name,
        &cfg.account_id,
    )
    .map_err(|e| e.to_string())?;

    let mut prov_state = persistence.load().await.map_err(|e| e.to_string())?;
    let entries = claria_provisioner::plan(&syncers, &prov_state)
        .await
        .map_err(|e| e.to_string())?;

    claria_provisioner::execute(&entries, &syncers, &mut prov_state, &persistence)
        .await
        .map_err(|e| e.to_string())?;

    // Re-plan to show updated state
    claria_provisioner::plan(&syncers, &prov_state)
        .await
        .map_err(|e| e.to_string())
}

/// Destroy all managed resources. Returns nothing on success.
#[tauri::command]
#[specta::specta]
pub async fn destroy(
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let manifest = claria_provisioner::build_manifest(
        &cfg.account_id,
        &cfg.system_name,
        &cfg.region,
    );
    let syncers = claria_provisioner::build_syncers(&sdk_config, &manifest);
    let persistence = claria_provisioner::build_persistence(
        &sdk_config,
        &cfg.system_name,
        &cfg.account_id,
    )
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
pub async fn reset_provisioner_state(
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let persistence = claria_provisioner::build_persistence(
        &sdk_config,
        &cfg.system_name,
        &cfg.account_id,
    )
    .map_err(|e| e.to_string())?;
    persistence.delete().await.map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Client commands — CRUD backed by S3
// ---------------------------------------------------------------------------

/// Helper: derive bucket name from config (same convention as provisioner).
fn bucket_name(cfg: &ClariaConfig) -> String {
    format!("{}-{}-data", cfg.account_id, cfg.system_name)
}

/// List all client records from S3.
///
/// Loads each `clients/{id}.json` object, deserializes the Client, and
/// returns summaries sorted by most recently created first.
#[tauri::command]
#[specta::specta]
pub async fn list_clients(
    state: State<'_, DesktopState>,
) -> Result<Vec<ClientSummary>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    let keys = claria_storage::objects::list_objects(&s3, &bucket, claria_core::s3_keys::CLIENTS_PREFIX)
        .await
        .map_err(|e| e.to_string())?;

    let mut clients: Vec<ClientSummary> = Vec::new();

    for key in &keys {
        let output = match claria_storage::objects::get_object(&s3, &bucket, key).await {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(key, error = %e, "skipping unreadable client object");
                continue;
            }
        };

        let client: claria_core::models::client::Client = match serde_json::from_slice(&output.body) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(key, error = %e, "skipping unparseable client object");
                continue;
            }
        };

        clients.push(ClientSummary {
            id: client.id.to_string(),
            name: client.name,
            created_at: client.created_at.to_string(),
        });
    }

    // Sort by created_at descending (most recent first).
    clients.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(clients)
}

/// Create a new client record in S3.
#[tauri::command]
#[specta::specta]
pub async fn create_client(
    state: State<'_, DesktopState>,
    name: String,
) -> Result<ClientSummary, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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

/// Delete a client and all associated data (record files, chat history).
#[tauri::command]
#[specta::specta]
pub async fn delete_client(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    // Delete all record files (includes chat history, sidecars, etc.)
    let records_prefix = claria_core::s3_keys::client_records_prefix(id);
    let deleted = claria_storage::objects::delete_objects_by_prefix(&s3, &bucket, &records_prefix)
        .await
        .map_err(|e| e.to_string())?;

    // Delete the client JSON itself.
    let client_key = claria_core::s3_keys::client(id);
    claria_storage::objects::delete_object(&s3, &bucket, &client_key)
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!(client_id = %id, deleted_records = deleted, "client deleted");

    Ok(())
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

/// List files in a client's record, excluding sidecar `.text` files.
#[tauri::command]
#[specta::specta]
pub async fn list_record_files(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<Vec<RecordFile>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let prefix = claria_core::s3_keys::client_records_prefix(id);

    let objects = claria_storage::objects::list_objects_with_metadata(&s3, &bucket, &prefix)
        .await
        .map_err(|e| e.to_string())?;

    // Collect all keys into a set so we can check for base files when filtering sidecars.
    let all_keys: std::collections::HashSet<&str> =
        objects.iter().map(|o| o.key.as_str()).collect();

    let files: Vec<RecordFile> = objects
        .iter()
        .filter(|obj| {
            // Hide sidecar files: keys ending in `.text` where the base file exists.
            if let Some(base) = obj.key.strip_suffix(".text") {
                return !all_keys.contains(base);
            }
            true
        })
        .filter_map(|obj| {
            // Strip the prefix to get just the filename.
            let filename = obj.key.strip_prefix(&prefix)?;
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

    Ok(files)
}

/// Upload a file to a client's record from a local file path.
///
/// If the file is a PDF or DOCX, a sidecar `.text` file is generated
/// via Bedrock document text extraction and uploaded alongside.
#[tauri::command]
#[specta::specta]
pub async fn upload_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    file_path: String,
) -> Result<RecordFile, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let path = std::path::Path::new(&file_path);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Invalid file path".to_string())?;

    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;
    let file_size = bytes.len() as i32;

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
            Ok(text) => {
                claria_storage::objects::put_object(
                    &s3,
                    &bucket,
                    &sidecar_key,
                    text.into_bytes(),
                    Some("text/plain"),
                )
                .await
                .map_err(|e| e.to_string())?;

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
    } else if let Some(media_format) =
        claria_transcribe::media_format_for_extension(&extension)
    {
        let sidecar_key = format!("{key}.text");
        match claria_transcribe::transcribe_audio(&sdk_config, &bucket, &key, media_format).await
        {
            Ok(text) => {
                claria_storage::objects::put_object(
                    &s3,
                    &bucket,
                    &sidecar_key,
                    text.into_bytes(),
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

/// Delete a file from a client's record, including its sidecar if present.
#[tauri::command]
#[specta::specta]
pub async fn delete_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
pub async fn get_record_file_text(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
) -> Result<String, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let key = claria_core::s3_keys::client_record_file(id, &filename);
    claria_storage::objects::put_object(&s3, &bucket, &key, content.into_bytes(), Some("text/plain"))
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
pub async fn list_record_context(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<Vec<RecordContext>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let prefix = claria_core::s3_keys::client_records_prefix(id);

    let keys = claria_storage::objects::list_objects(&s3, &bucket, &prefix)
        .await
        .map_err(|e| e.to_string())?;

    // Collect all keys into a set so we can identify sidecar files.
    let all_keys: std::collections::HashSet<&str> = keys.iter().map(|k| k.as_str()).collect();

    let mut context_files = Vec::new();

    for key in &keys {
        // Skip sidecar `.text` files — we read them via their parent.
        if let Some(base) = key.strip_suffix(".text")
            && all_keys.contains(base)
        {
            continue;
        }

        let filename = match key.strip_prefix(&prefix) {
            Some(f) if !f.is_empty() => f,
            _ => continue,
        };

        let text = if filename.ends_with(".txt") {
            // Plain text: read directly.
            match claria_storage::objects::get_object(&s3, &bucket, key).await {
                Ok(output) => String::from_utf8(output.body).ok(),
                Err(_) => None,
            }
        } else {
            // Other files: read the `.text` sidecar.
            let sidecar_key = format!("{key}.text");
            match claria_storage::objects::get_object(&s3, &bucket, &sidecar_key).await {
                Ok(output) => String::from_utf8(output.body).ok(),
                Err(_) => None,
            }
        };

        if let Some(text) = text {
            context_files.push(RecordContext {
                filename: filename.to_string(),
                text,
            });
        }
    }

    Ok(context_files)
}

/// Helper: load all record context for a client, converting to bedrock types.
async fn load_record_context(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: &str,
) -> Result<Vec<claria_bedrock::context::ContextFile>, String> {
    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let prefix = claria_core::s3_keys::client_records_prefix(id);

    let keys = claria_storage::objects::list_objects(s3, bucket, &prefix)
        .await
        .map_err(|e| e.to_string())?;

    let all_keys: std::collections::HashSet<&str> = keys.iter().map(|k| k.as_str()).collect();
    let mut files = Vec::new();

    for key in &keys {
        if let Some(base) = key.strip_suffix(".text")
            && all_keys.contains(base)
        {
            continue;
        }

        let filename = match key.strip_prefix(&prefix) {
            Some(f) if !f.is_empty() => f,
            _ => continue,
        };

        let text = if filename.ends_with(".txt") {
            match claria_storage::objects::get_object(s3, bucket, key).await {
                Ok(output) => String::from_utf8(output.body).ok(),
                Err(_) => None,
            }
        } else {
            let sidecar_key = format!("{key}.text");
            match claria_storage::objects::get_object(s3, bucket, &sidecar_key).await {
                Ok(output) => String::from_utf8(output.body).ok(),
                Err(_) => None,
            }
        };

        if let Some(text) = text {
            files.push(claria_bedrock::context::ContextFile {
                filename: filename.to_string(),
                text,
            });
        }
    }

    Ok(files)
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
fn resolve_prompt(name: &str) -> Result<(&'static str, Option<&'static str>, &'static str), String> {
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
                if let Err(e) =
                    claria_storage::objects::delete_object(s3, bucket, legacy).await
                {
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
pub async fn list_chat_models(
    state: State<'_, DesktopState>,
) -> Result<Vec<ChatModel>, String> {
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
pub async fn chat_message(
    state: State<'_, DesktopState>,
    client_id: String,
    model_id: String,
    messages: Vec<ChatMessage>,
    chat_id: Option<String>,
) -> Result<ChatResponse, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    let system_prompt = load_prompt(&s3, &bucket, "system-prompt").await?;

    // Load record context and prepend to the system prompt.
    let context_files = load_record_context(&s3, &bucket, &client_id).await?;
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

    let response_text =
        claria_bedrock::chat::chat_converse(&sdk_config, &model_id, &full_prompt, &bedrock_messages)
            .await
            .map_err(|e| e.to_string())?;

    // Resolve or generate the chat session ID.
    let chat_uuid: uuid::Uuid = match &chat_id {
        Some(id) => id.parse().map_err(|e: uuid::Error| e.to_string())?,
        None => uuid::Uuid::new_v4(),
    };
    let client_uuid: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    // Build the full message history including the new assistant response.
    let now = jiff::Timestamp::now();
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
            timestamp: now,
        })
        .collect();
    history_messages.push(claria_core::models::chat_history::ChatHistoryMessage {
        role: claria_core::models::chat_history::ChatHistoryRole::Assistant,
        content: response_text.clone(),
        timestamp: now,
    });

    let history = claria_core::models::chat_history::ChatHistory {
        id: chat_uuid,
        client_id: client_uuid,
        model_id: model_id.clone(),
        messages: history_messages,
        created_at: now,
        updated_at: now,
    };

    // Best-effort upload — don't fail the chat if persistence fails.
    let key = claria_core::s3_keys::chat_history(client_uuid, chat_uuid);
    match serde_json::to_vec_pretty(&history) {
        Ok(body) => {
            if let Err(e) =
                claria_storage::objects::put_object(&s3, &bucket, &key, body, Some("application/json"))
                    .await
            {
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
        content: response_text,
    })
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    let client_uuid: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let chat_uuid: uuid::Uuid = chat_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let key = claria_core::s3_keys::chat_history(client_uuid, chat_uuid);
    let output = claria_storage::objects::get_object(&s3, &bucket, &key)
        .await
        .map_err(|e| e.to_string())?;

    let history: claria_core::models::chat_history::ChatHistory =
        serde_json::from_slice(&output.body).map_err(|e| e.to_string())?;

    let messages = history
        .messages
        .into_iter()
        .map(|m| ChatMessage {
            role: match m.role {
                claria_core::models::chat_history::ChatHistoryRole::User => ChatRole::User,
                claria_core::models::chat_history::ChatHistoryRole::Assistant => ChatRole::Assistant,
            },
            content: m.content,
        })
        .collect();

    Ok(ChatHistoryDetail {
        chat_id: history.id.to_string(),
        model_id: history.model_id,
        messages,
        created_at: history.created_at.to_string(),
    })
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let all_deleted: std::collections::HashSet<&str> =
        entries.iter().map(|s| s.as_str()).collect();

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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
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
    let output =
        claria_storage::objects::get_object_version(&s3, &bucket, &key, &real.version_id)
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    let deleted = claria_storage::objects::list_deleted_objects(
        &s3,
        &bucket,
        claria_core::s3_keys::CLIENTS_PREFIX,
    )
    .await
    .map_err(|e| e.to_string())?;

    let mut clients = Vec::new();
    for d in &deleted {
        // Fetch the most recent real version to get the client name.
        let versions = claria_storage::objects::list_object_versions(&s3, &bucket, &d.key)
            .await
            .map_err(|e| e.to_string())?;

        // Find the most recent non-delete-marker version.
        let latest_real = versions.iter().find(|v| !v.is_delete_marker);
        let name = if let Some(v) = latest_real {
            if v.version_id.is_empty() {
                tracing::warn!(key = %d.key, "deleted client has empty version_id (pre-versioning object)");
                "Unknown".to_string()
            } else {
                match claria_storage::objects::get_object_version(
                    &s3,
                    &bucket,
                    &d.key,
                    &v.version_id,
                )
                .await
                {
                    Ok(output) => {
                        match serde_json::from_slice::<claria_core::models::client::Client>(
                            &output.body,
                        ) {
                            Ok(client) => client.name,
                            Err(e) => {
                                tracing::warn!(key = %d.key, version_id = %v.version_id, error = %e, "failed to deserialize deleted client JSON");
                                "Unknown".to_string()
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(key = %d.key, version_id = %v.version_id, error = %e, "failed to fetch deleted client version");
                        "Unknown".to_string()
                    }
                }
            }
        } else {
            tracing::warn!(key = %d.key, version_count = versions.len(), "no non-delete-marker version found for deleted client");
            "Unknown".to_string()
        };

        // Extract the UUID from the key (e.g. "clients/abc-123.json" → "abc-123")
        let id = d
            .key
            .strip_prefix(claria_core::s3_keys::CLIENTS_PREFIX)
            .and_then(|s| s.strip_suffix(".json"))
            .unwrap_or(&d.key)
            .to_string();

        clients.push(DeletedClient {
            id,
            name,
            deleted_at: d.last_modified.clone(),
            version_id: d.version_id.clone(),
        });
    }

    Ok(clients)
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
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    let id: uuid::Uuid = client_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let key = claria_core::s3_keys::client(id);

    // Find the most recent non-delete-marker version.
    let versions = claria_storage::objects::list_object_versions(&s3, &bucket, &key)
        .await
        .map_err(|e| e.to_string())?;
    let real = versions
        .iter()
        .find(|v| !v.is_delete_marker)
        .ok_or_else(|| format!("no restorable version found for {key}"))?;

    // Fetch that version's content and write it back as a new current version.
    let output =
        claria_storage::objects::get_object_version(&s3, &bucket, &key, &real.version_id)
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

    tracing::info!(client_id = %id, "deleted client restored");

    Ok(())
}

// ---------------------------------------------------------------------------
// Whisper model management + local transcription
// ---------------------------------------------------------------------------

/// Files required for the candle-based Whisper model.
const WHISPER_FILES: &[&str] = &["model.safetensors", "config.json", "tokenizer.json"];

/// Available Whisper model tiers.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum WhisperModelTier {
    BaseEn,
    Small,
    Turbo,
}

impl WhisperModelTier {
    fn hf_repo(&self) -> &'static str {
        match self {
            Self::BaseEn => "openai/whisper-base.en",
            Self::Small => "openai/whisper-small",
            Self::Turbo => "openai/whisper-large-v3-turbo",
        }
    }

    fn dir_name(&self) -> &'static str {
        match self {
            Self::BaseEn => "whisper-base-en",
            Self::Small => "whisper-small",
            Self::Turbo => "whisper-large-v3-turbo",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::BaseEn => "Good English",
            Self::Small => "Good English + Spanish",
            Self::Turbo => "Best Quality",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::BaseEn => "English-only model. Fastest inference, smallest download.",
            Self::Small => "Multilingual model with good English and Spanish support.",
            Self::Turbo => "Large-v3 Turbo (2024). Best multilingual accuracy with fast inference.",
        }
    }

    fn download_size(&self) -> &'static str {
        match self {
            Self::BaseEn => "~293 MB",
            Self::Small => "~967 MB",
            Self::Turbo => "~1.5 GB",
        }
    }

    fn tag(&self) -> &'static str {
        match self {
            Self::BaseEn => "base_en",
            Self::Small => "small",
            Self::Turbo => "turbo",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "base_en" => Some(Self::BaseEn),
            "small" => Some(Self::Small),
            "turbo" => Some(Self::Turbo),
            // Legacy: users who had "medium" active before the switch
            "medium" => Some(Self::Turbo),
            _ => None,
        }
    }

    fn all() -> &'static [WhisperModelTier] {
        &[Self::BaseEn, Self::Small, Self::Turbo]
    }
}

/// Info about a Whisper model tier (status, size, path, whether active).
/// Known tiers have `tier: Some(...)`. Orphan directories on disk that don't
/// match any known tier have `tier: None`.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct WhisperModelInfo {
    pub tier: Option<WhisperModelTier>,
    pub dir_name: String,
    pub label: String,
    pub description: String,
    pub download_size: String,
    pub downloaded: bool,
    pub model_size_bytes: Option<i32>,
    pub model_path: Option<String>,
    pub active: bool,
    /// Whether inference will use GPU acceleration (Metal on macOS).
    pub gpu_accelerated: bool,
}

/// Result from transcription, including detected language.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct TranscribeMemoResult {
    pub text: String,
    pub language: Option<String>,
}

fn whisper_models_base_dir() -> Result<std::path::PathBuf, String> {
    let base = dirs::data_dir().ok_or_else(|| "no data directory found".to_string())?;
    Ok(base.join("com.claria.desktop").join("models"))
}

fn whisper_model_dir(tier: &WhisperModelTier) -> Result<std::path::PathBuf, String> {
    Ok(whisper_models_base_dir()?.join(tier.dir_name()))
}

fn active_model_file() -> Result<std::path::PathBuf, String> {
    Ok(whisper_models_base_dir()?.join("active-whisper-model.txt"))
}

fn read_active_tier() -> Option<WhisperModelTier> {
    let path = active_model_file().ok()?;
    let tag = std::fs::read_to_string(path).ok()?;
    WhisperModelTier::from_tag(tag.trim())
}

fn write_active_tier(tier: &WhisperModelTier) -> Result<(), String> {
    let path = active_model_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create models dir: {e}"))?;
    }
    std::fs::write(&path, tier.tag()).map_err(|e| format!("write active model: {e}"))
}

fn clear_active_tier() {
    if let Ok(path) = active_model_file() {
        let _ = std::fs::remove_file(path);
    }
}

fn is_tier_downloaded(tier: &WhisperModelTier) -> bool {
    if let Ok(dir) = whisper_model_dir(tier) {
        WHISPER_FILES.iter().all(|f| dir.join(f).exists())
    } else {
        false
    }
}

/// Recursively sum the size of all files in a directory.
fn dir_size_bytes(path: &std::path::Path) -> Option<i32> {
    fn walk(dir: &std::path::Path, total: &mut u64) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, total);
                } else if let Ok(meta) = path.metadata() {
                    *total += meta.len();
                }
            }
        }
    }
    let mut total = 0u64;
    walk(path, &mut total);
    if total > 0 { Some(total as i32) } else { None }
}

/// Resolve the effective active tier: explicit selection, or auto-pick the
/// first downloaded model.
fn effective_active_tier() -> Option<WhisperModelTier> {
    if let Some(tier) = read_active_tier()
        && is_tier_downloaded(&tier)
    {
        return Some(tier);
    }
    // Fallback: first downloaded tier
    WhisperModelTier::all()
        .iter()
        .find(|t| is_tier_downloaded(t))
        .cloned()
}

fn build_whisper_models_list() -> Result<Vec<WhisperModelInfo>, String> {
    let active = effective_active_tier();
    let base_dir = whisper_models_base_dir()?;
    let gpu = claria_whisper::is_gpu_available();

    // Collect known-tier dir names so we can detect orphans.
    let known_dir_names: std::collections::HashSet<&str> =
        WhisperModelTier::all().iter().map(|t| t.dir_name()).collect();

    let mut models = Vec::new();
    for tier in WhisperModelTier::all() {
        let downloaded = is_tier_downloaded(tier);
        let dir = whisper_model_dir(tier)?;
        let is_active = active
            .as_ref()
            .is_some_and(|a| a.tag() == tier.tag());
        models.push(WhisperModelInfo {
            tier: Some(tier.clone()),
            dir_name: tier.dir_name().to_string(),
            label: tier.label().to_string(),
            description: tier.description().to_string(),
            download_size: tier.download_size().to_string(),
            downloaded,
            model_size_bytes: if downloaded { dir_size_bytes(&dir) } else { None },
            model_path: if downloaded {
                Some(dir.to_string_lossy().to_string())
            } else {
                None
            },
            active: is_active,
            gpu_accelerated: gpu,
        });
    }

    // Scan for orphan directories (not matching any known tier).
    if base_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&base_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if known_dir_names.contains(name_str.as_ref()) {
                continue;
            }
            models.push(WhisperModelInfo {
                tier: None,
                dir_name: name_str.to_string(),
                label: name_str.to_string(),
                description: "Unknown model \u{2014} not managed by Claria. Safe to remove."
                    .to_string(),
                download_size: String::new(),
                downloaded: true,
                model_size_bytes: dir_size_bytes(&path),
                model_path: Some(path.to_string_lossy().to_string()),
                active: false,
                gpu_accelerated: gpu,
            });
        }
    }

    Ok(models)
}

/// List all Whisper model tiers with their download/active status.
#[tauri::command]
#[specta::specta]
pub async fn get_whisper_models() -> Result<Vec<WhisperModelInfo>, String> {
    build_whisper_models_list()
}

/// Download a specific Whisper model tier from Hugging Face.
#[tauri::command]
#[specta::specta]
pub async fn download_whisper_model(
    tier: WhisperModelTier,
) -> Result<Vec<WhisperModelInfo>, String> {
    let dir = whisper_model_dir(&tier)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create models dir: {e}"))?;

    let hf_base = format!(
        "https://huggingface.co/{}/resolve/main",
        tier.hf_repo()
    );

    tracing::info!(tier = tier.tag(), "downloading whisper model files");

    let dir_clone = dir.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        for filename in WHISPER_FILES {
            let url = format!("{hf_base}/{filename}");
            let dest = dir_clone.join(filename);
            let tmp = dir_clone.join(format!("{filename}.tmp"));

            tracing::info!(url = %url, file = %filename, "downloading");

            let resp = ureq::get(&url)
                .call()
                .map_err(|e| format!("download {filename} failed: {e}"))?;

            let mut reader = resp.into_body().into_reader();
            let mut file = std::fs::File::create(&tmp)
                .map_err(|e| format!("create temp file for {filename}: {e}"))?;
            std::io::copy(&mut reader, &mut file)
                .map_err(|e| format!("write {filename}: {e}"))?;

            std::fs::rename(&tmp, &dest)
                .map_err(|e| format!("finalize {filename}: {e}"))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("download task failed: {e}"))??;

    tracing::info!(tier = tier.tag(), "whisper model download complete");

    // Auto-activate if no model is currently active.
    if effective_active_tier().is_none() {
        write_active_tier(&tier)?;
    }

    build_whisper_models_list()
}

/// Delete a specific Whisper model tier and clear the in-memory cache if needed.
#[tauri::command]
#[specta::specta]
pub async fn delete_whisper_model(
    state: State<'_, DesktopState>,
    tier: WhisperModelTier,
) -> Result<Vec<WhisperModelInfo>, String> {
    // If deleting the active tier, clear the active selection and cached model.
    let was_active = effective_active_tier()
        .as_ref()
        .is_some_and(|a| a.tag() == tier.tag());
    if was_active {
        clear_active_tier();
        if let Ok(mut guard) = state.whisper.lock() {
            *guard = None;
        }
    }

    let dir = whisper_model_dir(&tier)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("failed to delete model: {e}"))?;
        tracing::info!(tier = tier.tag(), "whisper model deleted");
    }

    build_whisper_models_list()
}

/// Delete a model directory by name. Used for orphan directories that don't
/// match any known tier. Also works for known tiers as a fallback.
#[tauri::command]
#[specta::specta]
pub async fn delete_whisper_model_dir(
    state: State<'_, DesktopState>,
    dir_name: String,
) -> Result<Vec<WhisperModelInfo>, String> {
    // Guard against path traversal.
    if dir_name.contains('/') || dir_name.contains('\\') || dir_name.contains("..") {
        return Err("Invalid directory name".to_string());
    }

    let base = whisper_models_base_dir()?;
    let dir = base.join(&dir_name);

    // If this directory matches the active tier, clear it.
    if let Some(active) = effective_active_tier()
        && active.dir_name() == dir_name
    {
        clear_active_tier();
        if let Ok(mut guard) = state.whisper.lock() {
            *guard = None;
        }
    }

    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("failed to delete model directory: {e}"))?;
        tracing::info!(dir_name = %dir_name, "whisper model directory deleted");
    }

    build_whisper_models_list()
}

/// Set the active Whisper model tier. The tier must be downloaded.
#[tauri::command]
#[specta::specta]
pub async fn set_active_whisper_model(
    state: State<'_, DesktopState>,
    tier: WhisperModelTier,
) -> Result<Vec<WhisperModelInfo>, String> {
    if !is_tier_downloaded(&tier) {
        return Err(format!(
            "Model '{}' is not downloaded. Download it first.",
            tier.label()
        ));
    }

    write_active_tier(&tier)?;

    // Clear the cached model so the next transcription loads the new one.
    if let Ok(mut guard) = state.whisper.lock() {
        *guard = None;
    }

    tracing::info!(tier = tier.tag(), "active whisper model changed");

    build_whisper_models_list()
}

/// Transcribe PCM audio using the active Whisper model.
///
/// Accepts base64-encoded f32 PCM samples at 16 kHz mono. Returns the
/// transcript text and detected language. The model is loaded on first call
/// and cached in memory for subsequent calls.
#[tauri::command]
#[specta::specta]
pub async fn transcribe_memo(
    state: State<'_, DesktopState>,
    audio_pcm_base64: String,
) -> Result<TranscribeMemoResult, String> {
    use base64::Engine;

    let active_tier = effective_active_tier()
        .ok_or("No Whisper model is active. Download and activate one from Preferences.")?;
    let model_dir = whisper_model_dir(&active_tier)?;
    if !model_dir.join("model.safetensors").exists() {
        return Err("Whisper model files missing. Re-download from Preferences.".into());
    }

    let pcm_bytes = base64::engine::general_purpose::STANDARD
        .decode(&audio_pcm_base64)
        .map_err(|e| format!("base64 decode failed: {e}"))?;

    if pcm_bytes.len() % 4 != 0 {
        return Err("PCM data length is not a multiple of 4 bytes (expected f32 samples)".into());
    }

    let pcm_samples: Vec<f32> = pcm_bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let duration_secs = pcm_samples.len() as f64 / 16000.0;
    tracing::info!(
        samples = pcm_samples.len(),
        duration_secs = format!("{duration_secs:.1}"),
        tier = active_tier.tag(),
        "transcribe_memo: received audio chunk"
    );

    let whisper = state.whisper.clone();

    // Run transcription in a blocking task (CPU-intensive).
    tokio::task::spawn_blocking(move || {
        let mut guard = whisper
            .lock()
            .map_err(|e| format!("whisper lock poisoned: {e}"))?;

        // Load model on first use.
        if guard.is_none() {
            tracing::info!(tier = active_tier.tag(), "loading whisper model into memory (first use)");
            let model = claria_whisper::WhisperModel::load(&model_dir)
                .map_err(|e| e.to_string())?;
            *guard = Some(model);
        }

        let start = std::time::Instant::now();
        let result = guard
            .as_mut()
            .expect("model just loaded")
            .transcribe(&pcm_samples, None)
            .map_err(|e| e.to_string())?;

        tracing::info!(
            elapsed_ms = start.elapsed().as_millis() as u64,
            text_len = result.text.len(),
            language = ?result.language,
            "transcribe_memo: inference complete"
        );

        Ok(TranscribeMemoResult {
            text: result.text,
            language: result.language,
        })
    })
    .await
    .map_err(|e| format!("transcription task failed: {e}"))?
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

            let tag = body["tag_name"]
                .as_str()
                .ok_or("missing tag_name")?;
            let latest = tag.strip_prefix('v').unwrap_or(tag).to_string();
            let release_url = body["html_url"]
                .as_str()
                .unwrap_or("https://github.com/claria-ai/claria/releases")
                .to_string();

            let update_available = latest.as_str() > current.as_str();

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