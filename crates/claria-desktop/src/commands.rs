use tauri::State;

use crate::aws::CallerIdentity;
use crate::config::{self, ClariaConfig, ConfigInfo, CredentialSource};
use crate::state::DesktopState;

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn has_config() -> Result<bool, String> {
    Ok(config::has_config())
}

#[tauri::command]
pub async fn load_config(
    state: State<'_, DesktopState>,
) -> Result<ConfigInfo, String> {
    let cfg = config::load_config().map_err(|e| e.to_string())?;
    let info = config::config_info(&cfg);

    // Cache in state
    let mut guard = state.config.lock().await;
    *guard = Some(cfg);

    Ok(info)
}

#[tauri::command]
pub async fn save_config(
    state: State<'_, DesktopState>,
    region: String,
    system_name: String,
    credentials: CredentialSource,
) -> Result<(), String> {
    let cfg = ClariaConfig {
        region,
        system_name,
        created_at: jiff::Timestamp::now(),
        credentials,
    };

    config::save_config(&cfg).map_err(|e| e.to_string())?;

    // Cache in state
    let mut guard = state.config.lock().await;
    *guard = Some(cfg);

    Ok(())
}

#[tauri::command]
pub async fn delete_config(
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    config::delete_config().map_err(|e| e.to_string())?;

    let mut guard = state.config.lock().await;
    *guard = None;

    Ok(())
}

// ---------------------------------------------------------------------------
// Credential commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn validate_credentials(
    region: String,
    credentials: CredentialSource,
) -> Result<CallerIdentity, String> {
    let sdk_config = crate::aws::build_aws_config(&region, &credentials).await;
    crate::aws::validate_credentials(&sdk_config)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_aws_profiles() -> Result<Vec<String>, String> {
    Ok(crate::aws::list_aws_profiles())
}

// ---------------------------------------------------------------------------
// Provisioner commands (stubbed — TODO: wire up claria-provisioner)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn scan_resources(
    _state: State<'_, DesktopState>,
) -> Result<serde_json::Value, String> {
    Err("not yet implemented: provisioner integration pending".to_string())
}

#[tauri::command]
pub async fn preview_plan(
    _state: State<'_, DesktopState>,
) -> Result<serde_json::Value, String> {
    Err("not yet implemented: provisioner integration pending".to_string())
}

#[tauri::command]
pub async fn provision(
    _state: State<'_, DesktopState>,
) -> Result<serde_json::Value, String> {
    Err("not yet implemented: provisioner integration pending".to_string())
}

#[tauri::command]
pub async fn destroy(
    _state: State<'_, DesktopState>,
) -> Result<(), String> {
    Err("not yet implemented: provisioner integration pending".to_string())
}
