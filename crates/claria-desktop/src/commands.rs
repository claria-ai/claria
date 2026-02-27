use serde::{Deserialize, Serialize};
use tauri::State;

use claria_desktop::config::{self, ClariaConfig, ConfigInfo, CredentialSource};
use claria_provisioner::account_setup::{
    AccessKeyInfo, AssumeRoleResult, BootstrapResult, CredentialAssessment, CredentialClass,
    StepStatus,
};
use claria_provisioner::{Plan, ScanResult};

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
// Provisioner commands — scan, plan, provision, destroy
// ---------------------------------------------------------------------------

/// Helper: load the saved config and build an SDK config from it.
///
/// Returns `(ClariaConfig, SdkConfig)`. Errors if no config is saved yet.
async fn load_sdk_config(
    state: &State<'_, DesktopState>,
) -> Result<(ClariaConfig, aws_config::SdkConfig), String> {
    let guard = state.config.lock().await;
    let cfg = guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "No config loaded. Complete setup first.".to_string())?;
    drop(guard);

    let sdk_config =
        claria_desktop::aws::build_aws_config(&cfg.region, &cfg.credentials).await;
    Ok((cfg, sdk_config))
}

/// Scan all managed AWS resources and return their current state.
///
/// This is a read-only operation — no resources are created or modified.
/// The frontend renders the results as a status table before prompting
/// the operator to review a plan.
#[tauri::command]
#[specta::specta]
pub async fn scan_resources(
    state: State<'_, DesktopState>,
) -> Result<Vec<ScanResult>, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let resources = claria_provisioner::build_resources(&sdk_config, &cfg.system_name, &cfg.account_id);

    let results = claria_provisioner::scan(&resources).await;
    Ok(results)
}

/// Scan resources, compare against persisted state, and return a four-bucket
/// plan (ok / modify / create / delete) without executing anything.
///
/// The frontend renders the plan for operator review before provisioning.
#[tauri::command]
#[specta::specta]
pub async fn preview_plan(
    state: State<'_, DesktopState>,
) -> Result<Plan, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let resources = claria_provisioner::build_resources(&sdk_config, &cfg.system_name, &cfg.account_id);
    let persistence = claria_provisioner::build_persistence(&sdk_config, &cfg.system_name, &cfg.account_id)
        .map_err(|e| e.to_string())?;

    let prov_state = persistence.load().await.map_err(|e| e.to_string())?;
    let scan_results = claria_provisioner::scan(&resources).await;
    let plan = claria_provisioner::build_plan(&prov_state, &scan_results, &resources);

    Ok(plan)
}

/// Execute the full scan → plan → execute pipeline.
///
/// Returns the plan that was executed so the frontend can show a summary.
/// State is flushed to local disk + S3 after each resource action.
#[tauri::command]
#[specta::specta]
pub async fn provision(
    state: State<'_, DesktopState>,
) -> Result<Plan, String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let resources = claria_provisioner::build_resources(&sdk_config, &cfg.system_name, &cfg.account_id);
    let persistence = claria_provisioner::build_persistence(&sdk_config, &cfg.system_name, &cfg.account_id)
        .map_err(|e| e.to_string())?;

    let mut prov_state = persistence.load().await.map_err(|e| e.to_string())?;
    let scan_results = claria_provisioner::scan(&resources).await;
    let plan = claria_provisioner::build_plan(&prov_state, &scan_results, &resources);

    if plan.has_changes() {
        claria_provisioner::execute_plan(&plan, &resources, &mut prov_state, &persistence)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(plan)
}

/// Destroy all managed resources and clear provisioner state.
///
/// The operator's config is NOT deleted — only the AWS resources and
/// the provisioner state file. The operator can re-provision later.
#[tauri::command]
#[specta::specta]
pub async fn destroy(
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let (cfg, sdk_config) = load_sdk_config(&state).await?;
    let resources = claria_provisioner::build_resources(&sdk_config, &cfg.system_name, &cfg.account_id);
    let persistence = claria_provisioner::build_persistence(&sdk_config, &cfg.system_name, &cfg.account_id)
        .map_err(|e| e.to_string())?;

    claria_provisioner::destroy(&persistence, &resources)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
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
// Chat commands — delegates to claria-bedrock
// ---------------------------------------------------------------------------

/// Specta type mirroring `claria_bedrock::chat::ChatModel`.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ChatModel {
    pub model_id: String,
    pub name: String,
}

/// System prompt for client intake chat.
const CHAT_SYSTEM_PROMPT: &str = "\
You are a clinical assistant helping a psychologist set up a new client record. \
Help gather relevant intake information such as the client's presenting concerns, \
referral source, relevant history, and initial observations. \
Be professional, empathetic, and concise. Ask clarifying questions when needed. \
Do not provide diagnoses or treatment recommendations — your role is to help \
organize and document the intake information.";

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
/// with each request so the model has context.
#[tauri::command]
#[specta::specta]
pub async fn chat_message(
    state: State<'_, DesktopState>,
    model_id: String,
    messages: Vec<ChatMessage>,
) -> Result<String, String> {
    let (_cfg, sdk_config) = load_sdk_config(&state).await?;

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

    claria_bedrock::chat::chat_converse(&sdk_config, &model_id, CHAT_SYSTEM_PROMPT, &bedrock_messages)
        .await
        .map_err(|e| e.to_string())
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