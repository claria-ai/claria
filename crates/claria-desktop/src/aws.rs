use std::path::PathBuf;

use crate::config::CredentialSource;

/// Build an `SdkConfig` from a region and credential source.
///
/// This is the only place in the desktop app that knows how to translate
/// a `CredentialSource` (our config-level type) into an AWS SDK config.
/// All AWS business logic lives in the provisioner — we just build the
/// config and hand it over.
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
            session_token,
        } => {
            builder = builder.credentials_provider(aws_sdk_sts::config::Credentials::new(
                access_key_id,
                secret_access_key,
                session_token.clone(),
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