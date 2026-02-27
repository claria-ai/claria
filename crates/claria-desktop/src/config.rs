use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClariaConfig {
    pub region: String,
    pub system_name: String,
    pub account_id: String,
    pub created_at: jiff::Timestamp,
    pub credentials: CredentialSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialSource {
    Inline {
        access_key_id: String,
        secret_access_key: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        session_token: Option<String>,
    },
    Profile {
        profile_name: String,
    },
    DefaultChain,
}

/// Redacted config info safe to send to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ConfigInfo {
    pub region: String,
    pub system_name: String,
    pub account_id: String,
    pub created_at: String,
    pub credential_type: String,
    pub profile_name: Option<String>,
    pub access_key_hint: Option<String>,
}

fn config_dir() -> eyre::Result<PathBuf> {
    let base = dirs::config_dir().ok_or_else(|| eyre::eyre!("no config directory found"))?;
    Ok(base.join("com.claria.desktop"))
}

fn config_path() -> eyre::Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

pub fn has_config() -> bool {
    config_path().map(|p| p.exists()).unwrap_or(false)
}

pub fn load_config() -> eyre::Result<ClariaConfig> {
    let path = config_path()?;
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| eyre::eyre!("failed to read config at {}: {e}", path.display()))?;
    let config: ClariaConfig = serde_json::from_str(&contents)?;
    Ok(config)
}

pub fn save_config(config: &ClariaConfig) -> eyre::Result<()> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)?;

    let path = dir.join("config.json");
    let json = serde_json::to_string_pretty(config)?;

    // Write to a temp file then rename for atomicity
    let tmp_path = dir.join("config.json.tmp");
    std::fs::write(&tmp_path, json.as_bytes())?;

    // Set restrictive permissions on Unix before renaming
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
    }

    std::fs::rename(&tmp_path, &path)?;

    tracing::info!(path = %path.display(), "config saved");
    Ok(())
}

pub fn delete_config() -> eyre::Result<()> {
    let path = config_path()?;
    if path.exists() {
        std::fs::remove_file(&path)?;
        tracing::info!(path = %path.display(), "config deleted");
    }
    Ok(())
}

pub fn config_info(config: &ClariaConfig) -> ConfigInfo {
    let (credential_type, profile_name, access_key_hint) = match &config.credentials {
        CredentialSource::Inline {
            access_key_id,
            session_token,
            ..
        } => {
            let cred_type = if session_token.is_some() {
                "temporary".to_string()
            } else {
                "inline".to_string()
            };
            let hint = redact_access_key(access_key_id);
            (cred_type, None, Some(hint))
        }
        CredentialSource::Profile { profile_name } => {
            ("profile".to_string(), Some(profile_name.clone()), None)
        }
        CredentialSource::DefaultChain => ("default_chain".to_string(), None, None),
    };

    ConfigInfo {
        region: config.region.clone(),
        system_name: config.system_name.clone(),
        account_id: config.account_id.clone(),
        created_at: config.created_at.to_string(),
        credential_type,
        profile_name,
        access_key_hint,
    }
}

fn redact_access_key(key: &str) -> String {
    if key.len() <= 8 {
        return "****".to_string();
    }
    let prefix = &key[..4];
    let suffix = &key[key.len() - 4..];
    format!("{prefix}...{suffix}")
}
