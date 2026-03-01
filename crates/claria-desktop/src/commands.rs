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
        match claria_bedrock::extract::extract_document_text(
            &sdk_config,
            EXTRACTION_MODEL_ID,
            &bytes,
            filename,
            format,
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

    // Best-effort delete of the sidecar.
    let sidecar_key = format!("{key}.text");
    let _ = claria_storage::objects::delete_object(&s3, &bucket, &sidecar_key).await;

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

/// Load the system prompt from S3, falling back to the hardcoded default.
async fn load_system_prompt(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
) -> Result<String, String> {
    match claria_storage::objects::get_object(s3, bucket, claria_core::s3_keys::SYSTEM_PROMPT).await {
        Ok(output) => String::from_utf8(output.body).map_err(|e| e.to_string()),
        Err(claria_storage::error::StorageError::NotFound { .. }) => {
            Ok(DEFAULT_SYSTEM_PROMPT.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
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

    let system_prompt = load_system_prompt(&s3, &bucket).await?;

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
// System prompt commands — editable prompt stored in S3
// ---------------------------------------------------------------------------

/// Get the current system prompt.
///
/// Returns the custom prompt from S3 if one exists, otherwise returns the
/// built-in default.
#[tauri::command]
#[specta::specta]
pub async fn get_system_prompt(
    state: State<'_, DesktopState>,
) -> Result<String, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    load_system_prompt(&s3, &bucket).await
}

/// Save a custom system prompt to S3.
///
/// Overwrites any previously saved prompt. The new prompt takes effect on
/// the next chat message.
#[tauri::command]
#[specta::specta]
pub async fn save_system_prompt(
    state: State<'_, DesktopState>,
    content: String,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    claria_storage::objects::put_object(
        &s3,
        &bucket,
        claria_core::s3_keys::SYSTEM_PROMPT,
        content.into_bytes(),
        Some("text/markdown"),
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Delete the custom system prompt from S3, reverting to the built-in default.
#[tauri::command]
#[specta::specta]
pub async fn delete_system_prompt(
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let s3 = aws_sdk_s3::Client::new(&sdk_config);
    let bucket = bucket_name(&cfg);

    claria_storage::objects::delete_object(&s3, &bucket, claria_core::s3_keys::SYSTEM_PROMPT)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}