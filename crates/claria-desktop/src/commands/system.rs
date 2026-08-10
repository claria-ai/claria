//! Update checks and shell/URL helpers.

use serde::{Deserialize, Serialize};

use super::{CommandError, run};

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
    run("check_for_updates", async {
        let current = env!("CARGO_PKG_VERSION").to_string();

        let result: Result<UpdateCheck, CommandError> = tokio::task::spawn_blocking({
            let current = current.clone();
            move || {
                let agent =
                    claria_desktop::http::ureq_agent(Some(std::time::Duration::from_secs(5)));
                let resp = agent
                    .get("https://api.github.com/repos/claria-ai/claria/releases/latest")
                    .header("User-Agent", "claria-desktop")
                    .header("Accept", "application/vnd.github+json")
                    .call()
                    .map_err(|e| CommandError::Msg(format!("{e}")))?;

                let body_str = resp
                    .into_body()
                    .read_to_string()
                    .map_err(|e| CommandError::Msg(e.to_string()))?;
                let body: serde_json::Value = serde_json::from_str(&body_str)?;

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
        .map_err(|e| CommandError::Msg(format!("update check task failed: {e}")))?;

        // On error, return a safe default instead of propagating.
        Ok(result.unwrap_or(UpdateCheck {
            current_version: current.clone(),
            latest_version: current,
            update_available: false,
            release_url: "https://github.com/claria-ai/claria/releases".to_string(),
        }))
    })
    .await
}

/// Open the application's log folder in the OS file manager, creating it if
/// file logging has not produced anything yet.
#[tauri::command]
#[specta::specta]
pub async fn reveal_log_folder() -> Result<(), String> {
    run("reveal_log_folder", async {
        let dir = claria_desktop::logging::app_log_dir().ok_or_else(|| {
            CommandError::Msg("This platform has no application log directory.".to_string())
        })?;
        std::fs::create_dir_all(&dir).map_err(|e| CommandError::Msg(e.to_string()))?;

        #[cfg(target_os = "macos")]
        let opener = "open";
        #[cfg(target_os = "windows")]
        let opener = "explorer";
        #[cfg(all(unix, not(target_os = "macos")))]
        let opener = "xdg-open";

        std::process::Command::new(opener)
            .arg(&dir)
            .spawn()
            .map_err(|e| CommandError::Msg(e.to_string()))?;
        Ok(())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn open_url(url: String) -> Result<(), String> {
    run("open_url", async {
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(CommandError::Msg(
                "URL must start with http:// or https://".into(),
            ));
        }

        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg(&url)
                .spawn()
                .map_err(|e| CommandError::Msg(e.to_string()))?;
        }

        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd")
                .args(["/c", "start", "", &url])
                .spawn()
                .map_err(|e| CommandError::Msg(e.to_string()))?;
        }

        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("xdg-open")
                .arg(&url)
                .spawn()
                .map_err(|e| CommandError::Msg(e.to_string()))?;
        }

        Ok(())
    })
    .await
}
