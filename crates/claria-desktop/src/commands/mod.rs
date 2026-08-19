//! Tauri command surface, one module per domain.
//!
//! Every command follows the same shape: a thin `#[tauri::command]` wrapper
//! whose body runs inside [`run`], which flattens the rich [`CommandError`]
//! exactly once at the boundary — logging the full chain, then prefixing the
//! operation name onto the string the frontend receives.

pub mod billing;
pub mod chat;
pub mod clients;
pub mod config;
pub mod console;
pub mod findings;
pub mod plan;
pub mod prompts;
pub mod provision;
pub mod records;
pub mod report;
pub mod runs;
pub mod streams;
pub mod system;
pub mod transcribe;
pub mod versions;

pub use billing::*;
pub use chat::*;
pub use clients::*;
pub use config::*;
pub use console::*;
pub use findings::*;
pub use plan::*;
pub use prompts::*;
pub use provision::*;
pub use records::*;
pub use report::*;
pub use runs::*;
pub use streams::*;
pub use system::*;
pub use transcribe::*;
pub use versions::*;

use tauri::State;

use claria_desktop::config::{self as config_file, ClariaConfig, CredentialSource};

use crate::state::DesktopState;

// ---------------------------------------------------------------------------
// CommandError — the one rich error type command bodies use
// ---------------------------------------------------------------------------

/// Every library-crate error a command body can hit, so `?` composes them
/// without per-call `.map_err(|e| e.to_string())`. Stringification happens
/// exactly once, in [`run`].
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Storage(#[from] claria_storage::error::StorageError),
    #[error(transparent)]
    Bedrock(#[from] claria_bedrock::error::BedrockError),
    #[error(transparent)]
    Records(#[from] claria_records::RecordsError),
    #[error(transparent)]
    Provisioner(#[from] claria_provisioner::ProvisionerError),
    #[error(transparent)]
    ReportPipeline(#[from] claria_report_pipeline::ReportPipelineError),
    #[error(transparent)]
    ReportStore(#[from] claria_report_store::ReportStoreError),
    #[error(transparent)]
    Transcribe(#[from] claria_transcribe::TranscribeError),
    #[error(transparent)]
    Billing(#[from] claria_billing::BillingError),
    #[error(transparent)]
    Docx(#[from] claria_docx::DocxError),
    #[error("invalid identifier: {0}")]
    Uuid(#[from] uuid::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// A user-facing message with no structured source (validation failures,
    /// config-layer `eyre` reports, ad-hoc conditions).
    #[error("{0}")]
    Msg(String),
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self::Msg(message)
    }
}

impl From<&str> for CommandError {
    fn from(message: &str) -> Self {
        Self::Msg(message.to_string())
    }
}

impl From<eyre::Report> for CommandError {
    fn from(report: eyre::Report) -> Self {
        // `:#` renders the full cause chain on one line.
        Self::Msg(format!("{report:#}"))
    }
}

/// Flatten a command body's error at the Tauri boundary.
///
/// This is the single place a command's rich error is logged (the full chain,
/// with the operation name) and the single place it is stringified for the
/// frontend, prefixed with the same operation name.
pub(crate) async fn run<T, F>(operation: &'static str, command: F) -> Result<T, String>
where
    F: Future<Output = Result<T, CommandError>>,
{
    flatten(operation, command.await)
}

/// [`run`] for the few synchronous commands.
pub(crate) fn flatten<T>(
    operation: &'static str,
    result: Result<T, CommandError>,
) -> Result<T, String> {
    result.map_err(|error| {
        tracing::error!(operation, error = ?error, "command failed");
        format!("{operation}: {error}")
    })
}

/// Parse a frontend-supplied UUID string.
pub(crate) fn parse_uuid(value: &str) -> Result<uuid::Uuid, CommandError> {
    Ok(value.parse::<uuid::Uuid>()?)
}

/// First `"{prefix} (N)"` (N = 1, 2, …) that does not collide with an
/// existing name. Shared by every auto-numbered-name surface ("Chat (N)",
/// "Writer Template (N)").
pub(crate) fn next_ordinal_name(prefix: &str, existing: &[String]) -> String {
    (1..)
        .map(|ordinal| format!("{prefix} ({ordinal})"))
        .find(|candidate| !existing.contains(candidate))
        .expect("an unused ordinal exists")
}

// ---------------------------------------------------------------------------
// CommandContext — config + AWS handles every S3-touching command needs
// ---------------------------------------------------------------------------

/// The per-command bundle of saved config, cached SDK config, cached S3
/// client, and derived bucket name.
pub(crate) struct CommandContext {
    pub cfg: ClariaConfig,
    pub sdk_config: aws_config::SdkConfig,
    pub s3: aws_sdk_s3::Client,
    pub bucket: String,
}

impl CommandContext {
    /// Load the saved config (in-memory first, disk fallback) and hand out
    /// the cached AWS clients for it. Errors when setup hasn't completed.
    pub async fn new(state: &State<'_, DesktopState>) -> Result<Self, CommandError> {
        let (cfg, sdk_config, s3) = load_sdk_config(state).await?;
        let bucket = bucket_name(&cfg);
        Ok(Self {
            cfg,
            sdk_config,
            s3,
            bucket,
        })
    }

    /// Build an [`claria_storage::audit::AuditEvent`] scoped to this
    /// account, ready for [`Self::record_audit`].
    ///
    /// `action` is an [`claria_storage::audit::actions`] constant, which
    /// carries its own category — see that module's docs for the taxonomy and
    /// the `details`-versus-`phi` contract.
    pub fn audit_event(
        &self,
        action: claria_storage::audit::Action,
        resource_type: &str,
        resource_id: impl Into<String>,
    ) -> claria_storage::audit::AuditEvent {
        claria_storage::audit::AuditEvent::new(
            action,
            resource_type,
            resource_id,
            self.cfg.account_id.clone(),
        )
        .with_app_version(env!("CARGO_PKG_VERSION"))
        .with_credential_id(self.credential_id())
    }

    /// Which credential is acting: the access key id for an inline
    /// credential, the profile name for a named profile, `None` for the
    /// default chain. Stamped once here rather than at each call site.
    ///
    /// Access key ids are identifiers, not secrets — CloudTrail records them
    /// on every call. The secret is never read.
    fn credential_id(&self) -> Option<String> {
        match &self.cfg.credentials {
            CredentialSource::Inline { access_key_id, .. } => Some(access_key_id.clone()),
            CredentialSource::Profile { profile_name } => Some(profile_name.clone()),
            CredentialSource::DefaultChain => None,
        }
    }

    /// Record an audit event against this context's bucket.
    ///
    /// See [`claria_desktop::audit::record`] for why this cannot fail the
    /// caller.
    pub async fn record_audit(&self, event: claria_storage::audit::AuditEvent) {
        claria_desktop::audit::record(&self.sdk_config, &self.bucket, event).await;
    }
}

/// Helper: derive bucket name from saved config.
pub(crate) fn bucket_name(cfg: &ClariaConfig) -> String {
    claria_core::s3_keys::bucket_name(&cfg.account_id, &cfg.system_name)
}

/// Helper: load the saved config and the cached AWS clients built from it.
///
/// If the in-memory state is empty, attempts to load from disk first. Errors
/// if no config is saved yet, or with the underlying reason a saved config
/// could not be loaded.
pub(crate) async fn load_sdk_config(
    state: &State<'_, DesktopState>,
) -> Result<(ClariaConfig, aws_config::SdkConfig, aws_sdk_s3::Client), CommandError> {
    let mut guard = state.config.lock().await;

    // Auto-load from disk if the in-memory state hasn't been populated yet.
    // `load_config` says "complete setup first" only when there is no config
    // file at all; a config that exists but cannot be loaded — a build too old
    // for its `config_version`, corrupt JSON, limits that fail validation —
    // carries its own reason all the way to the UI. Sending that user through
    // setup would have them overwrite a config this build merely failed to
    // parse.
    let cfg = match guard.as_ref() {
        Some(cfg) => cfg.clone(),
        None => {
            let cfg = config_file::load_config()?;
            *guard = Some(cfg.clone());
            cfg
        }
    };
    drop(guard);

    let (sdk_config, s3) = cached_aws(state, &cfg).await;
    Ok((cfg, sdk_config, s3))
}

/// Helper: the cached `SdkConfig` and S3 client for `cfg`'s region and
/// credentials, building and caching both on miss. Reuse is what keeps the
/// pooled HTTP connections warm across commands; the two are invalidated
/// together because the client is built from the config.
pub(crate) async fn cached_aws(
    state: &State<'_, DesktopState>,
    cfg: &ClariaConfig,
) -> (aws_config::SdkConfig, aws_sdk_s3::Client) {
    let mut sdk_guard = state.sdk_config.lock().await;
    if let Some(cached) = sdk_guard.as_ref()
        && cached.region == cfg.region
        && cached.credentials == cfg.credentials
    {
        return (cached.sdk_config.clone(), cached.s3.clone());
    }

    let sdk_config = claria_desktop::aws::build_aws_config(&cfg.region, &cfg.credentials).await;
    let s3 = claria_storage::client::from_config(&sdk_config);
    *sdk_guard = Some(crate::state::CachedSdkConfig {
        region: cfg.region.clone(),
        credentials: cfg.credentials.clone(),
        sdk_config: sdk_config.clone(),
        s3: s3.clone(),
    });
    (sdk_config, s3)
}

// ---------------------------------------------------------------------------
// Shared audit-detail helpers
// ---------------------------------------------------------------------------

/// Flat audit-detail fields for a Bedrock turn's token usage.
///
/// When Bedrock omitted the usage block (`None`), the event records
/// `usage_complete: false` instead of fabricated zero counts.
/// Resolve the user's model-tuning preferences against the capability table
/// for the chosen model. Unsupported knobs are dropped, never sent — the
/// capability table (not this call site) decides what each generation
/// accepts.
pub(crate) fn model_tuning_for(
    cfg: &ClariaConfig,
    model_id: &str,
) -> claria_bedrock::converse::ModelTuning {
    let capabilities = claria_core::model_id::ModelCapabilities::for_id(model_id);
    let preferences = cfg.model_tuning;
    claria_bedrock::converse::ModelTuning {
        adaptive_thinking: preferences.reasoning_enabled && capabilities.adaptive_thinking,
        effort: preferences
            .effort
            .filter(|_| capabilities.effort_parameter)
            .map(claria_desktop::config::EffortPreference::to_effort_level),
        temperature: preferences
            .temperature
            .filter(|_| capabilities.sampling_params),
    }
}

/// Overlay `extra`'s fields onto a shared audit-details object, so a command
/// adds what only it knows without restating the usage fields.
pub(crate) fn merge_details(base: &mut serde_json::Value, extra: serde_json::Value) {
    if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
}

pub(crate) fn usage_audit_details(
    model_id: &str,
    usage: Option<&claria_core::models::turn_usage::TurnUsage>,
    stop_reason: Option<&str>,
) -> serde_json::Value {
    let mut details = match usage {
        Some(usage) => serde_json::json!({
            "model_id": usage.model_id,
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "cache_read_input_tokens": usage.cache_read_input_tokens,
            "cache_write_input_tokens": usage.cache_write_input_tokens,
            "cache_ttl": usage.cache_ttl,
            "cost_usd": usage.cost_usd,
            "pricing_version": usage.pricing_version,
            "usage_complete": true,
            "app_version": env!("CARGO_PKG_VERSION"),
        }),
        None => serde_json::json!({
            "model_id": model_id,
            "usage_complete": false,
            "app_version": env!("CARGO_PKG_VERSION"),
        }),
    };
    if let Some(stop_reason) = stop_reason {
        details["stop_reason"] = serde_json::json!(stop_reason);
    }
    details
}
