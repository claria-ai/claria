//! Read-only view of the desktop's `config.json`.
//!
//! Only the fields this harness needs are declared; serde ignores the rest,
//! so a config carrying newer keys still loads. See the crate root for why
//! reading the desktop's config from here is a blessed exception, and for the
//! `config_version` caveat that comes with it.

use std::path::{Path, PathBuf};

use eyre::{Context, Result, eyre};
use serde::Deserialize;

/// The subset of `ClariaConfig` a headless writer run needs.
#[derive(Debug, Clone, Deserialize)]
pub struct EvalConfig {
    pub region: String,
    pub system_name: String,
    /// The 12-digit AWS account ID. Half of the bucket name, so an empty one
    /// is refused rather than defaulted — a security-scoping value fails
    /// closed.
    #[serde(default)]
    pub account_id: String,
    pub credentials: CredentialSource,
    /// The clinician's preferred writing model. The local copy only; the
    /// synced copy in S3 wins when both exist.
    #[serde(default)]
    pub preferred_model_id: Option<String>,
}

/// How the desktop authenticates to AWS. Mirrors
/// `claria_desktop::config::CredentialSource` field for field — the tag and
/// field names are the on-disk contract.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialSource {
    Inline {
        access_key_id: String,
        secret_access_key: String,
        #[serde(default)]
        session_token: Option<String>,
    },
    Profile {
        profile_name: String,
    },
    DefaultChain,
}

impl EvalConfig {
    /// The one bucket this account stores everything in.
    pub fn bucket(&self) -> Result<String> {
        if self.account_id.trim().is_empty() {
            return Err(eyre!(
                "the saved config has no AWS account ID, so the bucket name cannot be derived; \
                 open the desktop app once to backfill it"
            ));
        }
        if self.system_name.trim().is_empty() {
            return Err(eyre!("the saved config has no system name"));
        }
        Ok(claria_core::s3_keys::bucket_name(
            &self.account_id,
            &self.system_name,
        ))
    }
}

/// The directory the desktop keeps its config in. Derived exactly as
/// `claria_desktop::config::config_dir` does — replicated rather than called,
/// because depending on `claria-desktop` would drag Tauri into this binary.
pub fn desktop_config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().ok_or_else(|| eyre!("no config directory found"))?;
    Ok(base.join("com.claria.desktop"))
}

/// The default `--config` path.
pub fn default_config_path() -> Result<PathBuf> {
    Ok(desktop_config_dir()?.join("config.json"))
}

/// Load and parse a `config.json`. Never writes, never migrates.
pub fn load(path: &Path) -> Result<EvalConfig> {
    let bytes = std::fs::read(path)
        .wrap_err_with(|| format!("could not read the config at {}", path.display()))?;
    parse(&bytes)
}

/// Parse config bytes. Split out from [`load`] so tests can drive it from a
/// fixture without a real config directory.
pub fn parse(bytes: &[u8]) -> Result<EvalConfig> {
    serde_json::from_slice(bytes).wrap_err(
        "the config did not parse; this tool reads the desktop's config without running \
         migrations, so a config written by a newer build may need the desktop app opened once",
    )
}

/// Build the SDK config every library crate is handed.
///
/// Mirrors `claria_desktop::aws::build_aws_config`: the same region and
/// credential translation, and the same deliberately generous read timeout,
/// because a Bedrock unary call sends nothing until the model has finished
/// generating.
pub async fn build_aws_config(config: &EvalConfig) -> aws_config::SdkConfig {
    let timeouts = aws_config::timeout::TimeoutConfig::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .read_timeout(std::time::Duration::from_secs(15 * 60))
        .build();
    let stalled = aws_config::stalled_stream_protection::StalledStreamProtectionConfig::enabled()
        .grace_period(std::time::Duration::from_secs(10))
        .upload_enabled(true)
        .download_enabled(true)
        .build();

    let mut builder = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(config.region.clone()))
        .timeout_config(timeouts)
        .stalled_stream_protection(stalled);

    match &config.credentials {
        CredentialSource::Inline {
            access_key_id,
            secret_access_key,
            session_token,
        } => {
            builder = builder.credentials_provider(aws_credential_types::Credentials::new(
                access_key_id,
                secret_access_key,
                session_token.clone(),
                None,
                "claria-eval-config",
            ));
        }
        CredentialSource::Profile { profile_name } => {
            builder = builder.profile_name(profile_name);
        }
        CredentialSource::DefaultChain => {}
    }

    builder.load().await
}
