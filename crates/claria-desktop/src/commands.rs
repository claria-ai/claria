use tauri::State;

use claria_desktop::config::{self, ClariaConfig, ConfigInfo, CredentialSource};
use claria_provisioner::account_setup::{
    AccessKeyInfo, AssumeRoleResult, BootstrapResult, CredentialAssessment, CredentialClass,
    StepStatus,
};
use claria_provisioner::{Plan, ScanResult};

use crate::state::DesktopState;

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
    let cfg = config::load_config().map_err(|e| e.to_string())?;
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
    let result = claria_provisioner::bootstrap_account(
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
                region,
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