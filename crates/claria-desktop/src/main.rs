#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eyre::Result;

mod aws;
mod commands;
mod config;
mod state;

fn main() -> Result<()> {
    color_eyre::install()?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .manage(state::DesktopState::default())
        .invoke_handler(tauri::generate_handler![
            commands::has_config,
            commands::load_config,
            commands::save_config,
            commands::delete_config,
            commands::validate_credentials,
            commands::list_aws_profiles,
            commands::scan_resources,
            commands::preview_plan,
            commands::provision,
            commands::destroy,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| eyre::eyre!("tauri error: {e}"))?;

    Ok(())
}
