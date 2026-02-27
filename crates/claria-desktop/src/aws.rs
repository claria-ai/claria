use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::CredentialSource;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallerIdentity {
    pub account_id: String,
    pub arn: String,
    pub user_id: String,
}

/// Build an `SdkConfig` from a region and credential source.
pub async fn build_aws_config(
    region: &str,
    creds: &CredentialSource,
) -> aws_config::SdkConfig {
    let mut builder = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region.to_string()));

    match creds {
        CredentialSource::Inline {
            access_key_id,
            secret_access_key,
        } => {
            builder = builder.credentials_provider(aws_sdk_sts::config::Credentials::new(
                access_key_id,
                secret_access_key,
                None,
                None,
                "claria-config",
            ));
        }
        CredentialSource::Profile { profile_name } => {
            builder = builder.profile_name(profile_name);
        }
        CredentialSource::DefaultChain => {}
    }

    builder.load().await
}

/// Call STS GetCallerIdentity to validate credentials.
pub async fn validate_credentials(
    config: &aws_config::SdkConfig,
) -> eyre::Result<CallerIdentity> {
    let sts = aws_sdk_sts::Client::new(config);
    let resp = sts
        .get_caller_identity()
        .send()
        .await
        .map_err(|e| eyre::eyre!("STS GetCallerIdentity failed: {e}"))?;

    Ok(CallerIdentity {
        account_id: resp.account().unwrap_or_default().to_string(),
        arn: resp.arn().unwrap_or_default().to_string(),
        user_id: resp.user_id().unwrap_or_default().to_string(),
    })
}

/// Parse AWS profile names from `~/.aws/credentials` and `~/.aws/config`.
pub fn list_aws_profiles() -> Vec<String> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };

    let aws_dir = home.join(".aws");
    let mut profiles = std::collections::BTreeSet::new();

    // Parse [profile_name] from credentials file
    parse_ini_sections(&aws_dir.join("credentials"), &mut profiles, false);

    // Parse [profile name] from config file
    parse_ini_sections(&aws_dir.join("config"), &mut profiles, true);

    // Remove "default" — it's implicit
    profiles.remove("default");

    profiles.into_iter().collect()
}

/// Parse INI-style section headers from an AWS config/credentials file.
/// If `strip_profile_prefix` is true, strips the `profile ` prefix from
/// section names (as used in `~/.aws/config`).
fn parse_ini_sections(
    path: &PathBuf,
    profiles: &mut std::collections::BTreeSet<String>,
    strip_profile_prefix: bool,
) {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let mut name = trimmed[1..trimmed.len() - 1].trim().to_string();
            if strip_profile_prefix
                && let Some(stripped) = name.strip_prefix("profile ")
            {
                name = stripped.trim().to_string();
            }
            if !name.is_empty() {
                profiles.insert(name);
            }
        }
    }
}
