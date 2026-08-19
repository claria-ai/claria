//! Local configuration persistence for the desktop app.
//!
//! Credential storage: `config.json` holds the scoped IAM secret access key
//! in plaintext, protected only by the owner-only (0o600) permissions
//! [`save_config`] applies. Moving secrets into the OS keychain is tracked
//! in issue #73.

use std::path::{Path, PathBuf};

use claria_core::sensitive::Sensitive;
use serde::{Deserialize, Serialize};
use specta::Type;

/// Current config version. Bump this when adding fields or changing shape.
/// Each bump requires a corresponding entry in [`migrate`].
pub const CURRENT_VERSION: u32 = 13;

/// What the user is told when there is no config on disk at all.
///
/// Reserved for exactly that case. A config that exists but cannot be loaded
/// must surface its own reason instead: this message invites the user to
/// re-run setup, which overwrites the file, and a config the build merely
/// failed to parse is not one to overwrite.
pub const SETUP_REQUIRED: &str = "No config loaded. Complete setup first.";

/// Synced-preferences schema version. Independent of [`CURRENT_VERSION`]
/// because the synced subset lives in S3 and may be read by other machines'
/// builds.
pub const PREFERENCES_VERSION: u32 = 5;

fn default_prompt_caching_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClariaConfig {
    /// Schema version. Missing or 0 = pre-versioned config.
    #[serde(default)]
    pub config_version: u32,
    pub region: String,
    pub system_name: String,
    /// The 12-digit AWS account ID. Added in v1; older configs get an
    /// empty default that the `load_config` command backfills via STS.
    #[serde(default)]
    pub account_id: String,
    pub created_at: jiff::Timestamp,
    pub credentials: CredentialSource,
    /// The clinician's preferred chat model ID. Added in v2.
    #[serde(default)]
    pub preferred_model_id: Option<String>,
    /// Whether the user has completed Cost Explorer onboarding. Added in v3.
    #[serde(default)]
    pub cost_explorer_enabled: bool,
    /// Whether the user has enabled hourly-resolution cost data. Added in v4.
    #[serde(default)]
    pub hourly_cost_data: bool,
    /// Whether Bedrock prompt caching is enabled for chat. Added in v5.
    /// Default `true` — caching is a pure cost win on supported models and
    /// silently no-ops on models that don't honour it.
    #[serde(default = "default_prompt_caching_enabled")]
    pub prompt_caching_enabled: bool,
    /// Transcription defaults applied to drag-and-drop audio uploads and the
    /// wizard's pre-filled values. Added in v6.
    #[serde(default)]
    pub transcription: TranscriptionPreferences,
    /// Guardrails for the agentic document-writing loop. Added in v8.
    #[serde(default)]
    pub report_authoring: ReportAuthoringPreferences,
    /// Opt-in model tuning (adaptive thinking, effort, temperature). Added
    /// in v9. Defaults leave every request byte-identical to pre-v9 builds.
    #[serde(default)]
    pub model_tuning: ModelTuningPreferences,
    /// How incremental chat replies are delivered to the reader. Added in
    /// v10.
    #[serde(default)]
    pub chat_streaming: ChatStreamMode,
    /// How the sectioned drafting pipeline behaves: whether the plan waits
    /// for approval, and which models the supporting roles use. Added in v11.
    #[serde(default)]
    pub draft_pipeline: DraftPipelinePreferences,
    /// Auto-lock and PIN settings. Added in v13. Machine-local by design and
    /// deliberately absent from [`SyncedPreferences`]: the PIN guards this
    /// computer, and a hash that travelled to S3 would be a credential
    /// sitting in the bucket it protects.
    #[serde(default)]
    pub security: SecuritySettings,
}

fn default_auto_lock_timeout_minutes() -> u32 {
    crate::security::DEFAULT_TIMEOUT_MINUTES
}

/// How the app locks itself when the clinician walks away from it.
///
/// The PIN exists here only as an argon2id PHC string. It is wrapped in
/// [`Sensitive`] so that `{:?}` on a whole `ClariaConfig` — which happens in
/// error paths nobody writes deliberately — cannot put the hash into a
/// support export, and the struct does not derive [`Type`]: the frontend is
/// given the runtime lock state instead, which reports only whether a PIN
/// exists.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecuritySettings {
    /// Whether the idle timer, the wake check, and the startup lock are armed.
    #[serde(default)]
    pub auto_lock_enabled: bool,
    /// Minutes of inactivity before the session locks.
    #[serde(default = "default_auto_lock_timeout_minutes")]
    pub auto_lock_timeout_minutes: u32,
    /// Whether Touch ID / Windows Hello may answer the lock screen. The PIN
    /// is always accepted regardless.
    #[serde(default)]
    pub biometric_unlock_enabled: bool,
    /// argon2id PHC string for the unlock PIN, or `None` when none is set.
    #[serde(default)]
    pub pin_hash: Option<Sensitive<String>>,
}

impl Default for SecuritySettings {
    fn default() -> Self {
        Self {
            auto_lock_enabled: false,
            auto_lock_timeout_minutes: default_auto_lock_timeout_minutes(),
            biometric_unlock_enabled: false,
            pin_hash: None,
        }
    }
}

impl SecuritySettings {
    /// Whether a PIN is stored. The only thing about the PIN that may leave
    /// this struct.
    pub fn pin_set(&self) -> bool {
        self.pin_hash.is_some()
    }

    /// Whether the app should actually lock itself. Enabling auto-lock
    /// without a PIN would leave no way back in, so both are required.
    pub fn armed(&self) -> bool {
        self.auto_lock_enabled && self.pin_set()
    }
}

/// Whether a whole-report draft stops for the clinician between planning and
/// writing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PlanGateMode {
    /// The plan lands in the draft-run pane and waits to be read, edited, and
    /// approved. The default: a plan is cheap to fix and expensive to draft
    /// against once it is wrong.
    #[default]
    Gated,
    /// Drafting begins as soon as the plan lands.
    AutoStart,
}

/// Per-clinician settings for the sectioned drafting pipeline.
///
/// The two model IDs name the supporting roles only — the writer keeps its
/// own picker beside the draft. `None` means "let Claria choose", which
/// resolves through the capability table at call time rather than being
/// frozen into the config, so an account that gains a better model gets it
/// without anyone editing a setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DraftPipelinePreferences {
    #[serde(default)]
    pub plan_gate: PlanGateMode,
    #[serde(default)]
    pub planner_model_id: Option<String>,
    #[serde(default)]
    pub reviewer_model_id: Option<String>,
}

/// How much of a chat reply appears at a time.
///
/// Bedrock streams a few characters per frame. Passing every one of those to
/// the UI is what makes a reply twitch while it is being read, so the
/// default releases whole paragraphs; the other two settings are the
/// extremes on either side of it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChatStreamMode {
    /// Every delta, as fast as the service produces it.
    Token,
    /// A paragraph at a time.
    #[default]
    Paragraph,
    /// Nothing until the reply is complete.
    Off,
}

impl ChatStreamMode {
    pub fn to_pacing(self) -> claria_bedrock::pacing::StreamPacing {
        match self {
            Self::Token => claria_bedrock::pacing::StreamPacing::Token,
            Self::Paragraph => claria_bedrock::pacing::StreamPacing::Paragraph,
            Self::Off => claria_bedrock::pacing::StreamPacing::Off,
        }
    }
}

/// Opt-in model-tuning knobs. Every knob defaults to "send nothing", and
/// each is applied only on models whose capability-table entry accepts it —
/// see `commands::model_tuning_for`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct ModelTuningPreferences {
    /// Request adaptive thinking on models that support it (Claude 4.6+).
    #[serde(default)]
    pub reasoning_enabled: bool,
    /// Requested effort level; `None` leaves the model default (high).
    #[serde(default)]
    pub effort: Option<EffortPreference>,
    /// Sampling temperature, only sent to generations that accept it
    /// (through Claude 4.6); `None` leaves the model default.
    #[serde(default)]
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EffortPreference {
    Low,
    Medium,
    High,
    Max,
}

impl EffortPreference {
    pub fn to_effort_level(self) -> claria_bedrock::converse::EffortLevel {
        match self {
            Self::Low => claria_bedrock::converse::EffortLevel::Low,
            Self::Medium => claria_bedrock::converse::EffortLevel::Medium,
            Self::High => claria_bedrock::converse::EffortLevel::High,
            Self::Max => claria_bedrock::converse::EffortLevel::Max,
        }
    }
}

impl ModelTuningPreferences {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(temperature) = self.temperature
            && !(0.0..=1.0).contains(&temperature)
        {
            return Err("Temperature must be between 0.0 and 1.0.".to_string());
        }
        Ok(())
    }
}

/// Per-clinician guardrails for agentic document writing. These values sync
/// across machines. The report-pipeline crate validates the
/// relationship between the limits before they are saved or used.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct ReportAuthoringPreferences {
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: u32,
    #[serde(default = "default_max_converse_calls")]
    pub max_converse_calls: u32,
    #[serde(default = "default_max_tool_uses_per_response")]
    pub max_tool_uses_per_response: u32,
    #[serde(default = "default_max_retained_turns")]
    pub max_retained_turns: u32,
    #[serde(default = "default_writer_first_frame_timeout_secs")]
    pub writer_first_frame_timeout_secs: u32,
    #[serde(default = "default_writer_idle_timeout_secs")]
    pub writer_idle_timeout_secs: u32,
    #[serde(default = "default_writer_max_output_tokens")]
    pub writer_max_output_tokens: u32,
    #[serde(default = "default_analysis_first_frame_timeout_secs")]
    pub analysis_first_frame_timeout_secs: u32,
    #[serde(default = "default_analysis_idle_timeout_secs")]
    pub analysis_idle_timeout_secs: u32,
}

impl ReportAuthoringPreferences {
    pub fn limits(&self) -> Result<claria::ReportTurnLimits, String> {
        claria::ReportTurnLimits::try_new(
            self.max_tool_rounds,
            self.max_converse_calls,
            self.max_tool_uses_per_response,
            self.max_retained_turns,
        )
        .and_then(|limits| limits.with_runtime(self.runtime()))
        .map_err(|error| error.to_string())
    }

    /// The per-call runtime dials, in the shape the pipeline takes them.
    pub fn runtime(&self) -> claria::BedrockRuntimeLimits {
        claria::BedrockRuntimeLimits {
            writer_first_frame_timeout_secs: self.writer_first_frame_timeout_secs,
            writer_idle_timeout_secs: self.writer_idle_timeout_secs,
            writer_max_output_tokens: self.writer_max_output_tokens,
            analysis_first_frame_timeout_secs: self.analysis_first_frame_timeout_secs,
            analysis_idle_timeout_secs: self.analysis_idle_timeout_secs,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.limits().map(|_| ())
    }
}

impl Default for ReportAuthoringPreferences {
    fn default() -> Self {
        Self {
            max_tool_rounds: default_max_tool_rounds(),
            max_converse_calls: default_max_converse_calls(),
            max_tool_uses_per_response: default_max_tool_uses_per_response(),
            max_retained_turns: default_max_retained_turns(),
            writer_first_frame_timeout_secs: default_writer_first_frame_timeout_secs(),
            writer_idle_timeout_secs: default_writer_idle_timeout_secs(),
            writer_max_output_tokens: default_writer_max_output_tokens(),
            analysis_first_frame_timeout_secs: default_analysis_first_frame_timeout_secs(),
            analysis_idle_timeout_secs: default_analysis_idle_timeout_secs(),
        }
    }
}

fn default_max_tool_rounds() -> u32 {
    claria::DEFAULT_MAX_TOOL_ROUNDS
}

fn default_max_converse_calls() -> u32 {
    claria::DEFAULT_MAX_CONVERSE_CALLS
}

fn default_max_tool_uses_per_response() -> u32 {
    claria::DEFAULT_MAX_TOOL_USES_PER_RESPONSE
}

fn default_max_retained_turns() -> u32 {
    claria::DEFAULT_MAX_RETAINED_TURNS
}

fn default_writer_first_frame_timeout_secs() -> u32 {
    claria::DEFAULT_WRITER_FIRST_FRAME_TIMEOUT_SECS
}

fn default_writer_idle_timeout_secs() -> u32 {
    claria::DEFAULT_WRITER_IDLE_TIMEOUT_SECS
}

fn default_writer_max_output_tokens() -> u32 {
    claria::DEFAULT_WRITER_MAX_OUTPUT_TOKENS
}

fn default_analysis_first_frame_timeout_secs() -> u32 {
    claria::DEFAULT_ANALYSIS_FIRST_FRAME_TIMEOUT_SECS
}

fn default_analysis_idle_timeout_secs() -> u32 {
    claria::DEFAULT_ANALYSIS_IDLE_TIMEOUT_SECS
}

/// Per-clinician defaults for the transcription pipeline.
///
/// These flow into both the drag-and-drop fast path (used as-is) and the wizard
/// (pre-filled, user may override per-file). They sync across the clinician's
/// machines via the [`SyncedPreferences`] mechanism.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct TranscriptionPreferences {
    #[serde(default = "default_language")]
    pub default_language: TranscriptionLanguage,
    #[serde(default = "default_speaker_count")]
    pub default_speaker_count: u8,
    /// When true, English-only sessions route to Transcribe Medical (3x cost,
    /// clinical-vocabulary tuning, PHI tagging). Spanish and Mixed sessions
    /// always use Standard.
    #[serde(default)]
    pub use_medical_for_english: bool,
    /// When true, segments whose detected language is not English get an
    /// English translation rendered alongside the original via Bedrock.
    /// Default off — the primary user is bilingual; future users may want it.
    #[serde(default)]
    pub translate_to_english: bool,
    // TODO(vocab): re-add `custom_vocabulary_name: Option<String>` when Claria
    // gains a vocabulary-management surface. AWS treats Standard en-US,
    // Standard es-US, and Medical en-US vocabularies as three separate
    // resource types, so the shape will need to be a typed struct
    // (e.g. CustomVocabulary { standard_en, standard_es, medical_en }) rather
    // than a single string.
}

impl Default for TranscriptionPreferences {
    fn default() -> Self {
        Self {
            default_language: TranscriptionLanguage::English,
            default_speaker_count: 2,
            use_medical_for_english: false,
            translate_to_english: false,
        }
    }
}

fn default_language() -> TranscriptionLanguage {
    TranscriptionLanguage::English
}

fn default_speaker_count() -> u8 {
    2
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionLanguage {
    English,
    Spanish,
    Mixed,
}

/// The subset of [`ClariaConfig`] that follows a clinician across machines via
/// `_state/preferences.json` in S3. Machine-local fields (region, credentials,
/// account_id, system_name, created_at, config_version) are deliberately
/// excluded — they're deployment-identity and security-sensitive.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SyncedPreferences {
    pub preferences_version: u32,
    #[serde(default)]
    pub preferred_model_id: Option<String>,
    #[serde(default)]
    pub cost_explorer_enabled: bool,
    #[serde(default)]
    pub hourly_cost_data: bool,
    #[serde(default = "default_prompt_caching_enabled")]
    pub prompt_caching_enabled: bool,
    #[serde(default)]
    pub transcription: TranscriptionPreferences,
    #[serde(default)]
    pub report_authoring: ReportAuthoringPreferences,
    #[serde(default)]
    pub model_tuning: ModelTuningPreferences,
    #[serde(default)]
    pub chat_streaming: ChatStreamMode,
    #[serde(default)]
    pub draft_pipeline: DraftPipelinePreferences,
}

impl SyncedPreferences {
    /// Extract the syncable subset from a full local config.
    pub fn from_config(config: &ClariaConfig) -> Self {
        Self {
            preferences_version: PREFERENCES_VERSION,
            preferred_model_id: config.preferred_model_id.clone(),
            cost_explorer_enabled: config.cost_explorer_enabled,
            hourly_cost_data: config.hourly_cost_data,
            prompt_caching_enabled: config.prompt_caching_enabled,
            transcription: config.transcription.clone(),
            report_authoring: config.report_authoring.clone(),
            model_tuning: config.model_tuning,
            chat_streaming: config.chat_streaming,
            draft_pipeline: config.draft_pipeline.clone(),
        }
    }

    /// Overlay synced fields onto an in-memory config. Machine-local fields are
    /// left untouched.
    pub fn apply_to_config(&self, config: &mut ClariaConfig) {
        config.preferred_model_id = self.preferred_model_id.clone();
        config.cost_explorer_enabled = self.cost_explorer_enabled;
        config.hourly_cost_data = self.hourly_cost_data;
        config.prompt_caching_enabled = self.prompt_caching_enabled;
        config.transcription = self.transcription.clone();
        config.report_authoring = self.report_authoring.clone();
        config.model_tuning = self.model_tuning;
        config.chat_streaming = self.chat_streaming;
        config.draft_pipeline = self.draft_pipeline.clone();
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
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
    pub preferred_model_id: Option<String>,
    pub cost_explorer_enabled: bool,
    pub hourly_cost_data: bool,
    pub prompt_caching_enabled: bool,
    pub transcription: TranscriptionPreferences,
    pub report_authoring: ReportAuthoringPreferences,
    pub model_tuning: ModelTuningPreferences,
    pub chat_streaming: ChatStreamMode,
    pub draft_pipeline: DraftPipelinePreferences,
}

fn config_dir() -> eyre::Result<PathBuf> {
    let base = dirs::config_dir().ok_or_else(|| eyre::eyre!("no config directory found"))?;
    Ok(base.join("com.claria.desktop"))
}

fn config_path() -> eyre::Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

/// Machine-local directory for the provisioner's safety-net state copy.
///
/// Library crates never derive local paths (boundary rule): the desktop
/// resolves this directory and passes it into
/// `claria_provisioner::build_persistence`.
pub fn provisioner_state_dir(system_name: &str) -> eyre::Result<PathBuf> {
    Ok(config_dir()?.join(system_name))
}

pub fn has_config() -> bool {
    config_path().map(|p| p.exists()).unwrap_or(false)
}

/// Parse raw `config.json` contents: migrate to [`CURRENT_VERSION`],
/// deserialize, validate. Pure — it touches no files — so every way a config
/// can fail to load is reachable from a test.
///
/// Returns the version found on disk alongside the config, which the caller
/// needs to decide whether to write the migrated form back.
fn parse_config(contents: &str) -> eyre::Result<(ClariaConfig, u32)> {
    // Parse as raw JSON so we can run migrations before deserializing.
    let json: serde_json::Value = serde_json::from_str(contents)?;
    let on_disk_version = json
        .get("config_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let migrated = migrate(json, on_disk_version)?;
    let config: ClariaConfig = serde_json::from_value(migrated)?;
    config
        .report_authoring
        .validate()
        .map_err(|error| eyre::eyre!(error))?;

    Ok((config, on_disk_version))
}

/// Read and parse the config at `path`, distinguishing "there is no config"
/// from "there is one and it could not be loaded".
///
/// `Ok(None)` means the file does not exist. Every other failure — a
/// `config_version` newer than this build understands, corrupt JSON, limits
/// that no longer validate — comes back as an error carrying its own reason.
pub fn read_config_at(path: &Path) -> eyre::Result<Option<(ClariaConfig, u32)>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(eyre::eyre!(
                "failed to read config at {}: {error}",
                path.display()
            ));
        }
    };
    parse_config(&contents).map(Some)
}

/// The saved config, or the reason there isn't one to hand back.
///
/// A missing file yields [`SETUP_REQUIRED`]; a config that exists but cannot
/// be loaded yields the underlying reason, so a build too old for the config
/// on disk says so instead of sending the clinician back through setup.
pub fn load_config() -> eyre::Result<ClariaConfig> {
    load_config_at(&config_path()?)
}

/// [`load_config`] against an explicit path. Migrated configs are written
/// back to the same file so later loads skip the migration chain.
pub fn load_config_at(path: &Path) -> eyre::Result<ClariaConfig> {
    let Some((config, on_disk_version)) = read_config_at(path)? else {
        return Err(eyre::eyre!(SETUP_REQUIRED));
    };

    // Persist the migrated config so subsequent loads don't re-run migrations.
    if on_disk_version < CURRENT_VERSION {
        save_config_at(path, &config)?;
    }

    Ok(config)
}

/// Run sequential migrations from `from_version` up to [`CURRENT_VERSION`].
///
/// Each migration is a pure transform on the raw JSON value. Async work
/// (like STS calls to backfill `account_id`) lives in the Tauri command
/// layer, not here.
fn migrate(mut json: serde_json::Value, from_version: u32) -> eyre::Result<serde_json::Value> {
    if from_version > CURRENT_VERSION {
        return Err(eyre::eyre!(
            "config_version {from_version} is newer than this build supports ({CURRENT_VERSION}). \
             Please update Claria."
        ));
    }

    // v0 → v1: add account_id (empty string; backfilled by load_config command via STS)
    if from_version < 1 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        obj.entry("account_id")
            .or_insert(serde_json::Value::String(String::new()));
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(1.into()),
        );
        tracing::info!("migrated config v0 → v1 (added account_id)");
    }

    // v1 → v2: add preferred_model_id (null; clinician can set via Preferences)
    if from_version < 2 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        obj.entry("preferred_model_id")
            .or_insert(serde_json::Value::Null);
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(2.into()),
        );
        tracing::info!("migrated config v1 → v2 (added preferred_model_id)");
    }

    // v2 → v3: add cost_explorer_enabled (false; user enables via onboarding flow)
    if from_version < 3 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        obj.entry("cost_explorer_enabled")
            .or_insert(serde_json::Value::Bool(false));
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(3.into()),
        );
        tracing::info!("migrated config v2 → v3 (added cost_explorer_enabled)");
    }

    // v3 → v4: add hourly_cost_data (false; user enables via Preferences)
    if from_version < 4 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        obj.entry("hourly_cost_data")
            .or_insert(serde_json::Value::Bool(false));
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(4.into()),
        );
        tracing::info!("migrated config v3 → v4 (added hourly_cost_data)");
    }

    // v4 → v5: add prompt_caching_enabled (default true; user can toggle
    // via Preferences once that surface lands).
    if from_version < 5 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        obj.entry("prompt_caching_enabled")
            .or_insert(serde_json::Value::Bool(true));
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(5.into()),
        );
        tracing::info!("migrated config v4 → v5 (added prompt_caching_enabled)");
    }

    // v5 → v6: add transcription preferences (defaults: English, 2 speakers,
    // Standard engine).
    if from_version < 6 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        obj.entry("transcription").or_insert(serde_json::json!({
            "default_language": "english",
            "default_speaker_count": 2,
            "use_medical_for_english": false,
        }));
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(6.into()),
        );
        tracing::info!("migrated config v5 → v6 (added transcription preferences)");
    }

    // v6 → v7: add `translate_to_english` to transcription preferences
    // (default false; primary user is bilingual).
    if from_version < 7 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        if let Some(serde_json::Value::Object(transcription)) = obj.get_mut("transcription") {
            transcription
                .entry("translate_to_english")
                .or_insert(serde_json::Value::Bool(false));
        }
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(7.into()),
        );
        tracing::info!("migrated config v6 → v7 (added translate_to_english)");
    }

    // v7 → v8: add configurable report-authoring guardrails at ten times the
    // original fixed limits.
    if from_version < 8 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        obj.entry("report_authoring").or_insert(serde_json::json!({
            "max_tool_rounds": default_max_tool_rounds(),
            "max_converse_calls": default_max_converse_calls(),
            "max_tool_uses_per_response": default_max_tool_uses_per_response(),
            "max_retained_turns": default_max_retained_turns(),
        }));
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(8.into()),
        );
        tracing::info!("migrated config v7 → v8 (added report authoring preferences)");
    }

    if from_version < 9 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        obj.insert(
            "model_tuning".to_string(),
            serde_json::json!({
                "reasoning_enabled": false,
                "effort": null,
                "temperature": null,
            }),
        );
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(9.into()),
        );
        tracing::info!("migrated config v8 → v9 (added model tuning preferences)");
    }

    // v9 → v10: add the chat streaming cadence. Existing installs land on
    // the new paragraph default rather than the token-by-token behaviour
    // they had, which is the point of the setting.
    if from_version < 10 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        obj.entry("chat_streaming")
            .or_insert(serde_json::json!("paragraph"));
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(10.into()),
        );
        tracing::info!("migrated config v9 → v10 (added chat streaming mode)");
    }

    // v10 → v11: add the drafting-pipeline settings. The plan gate defaults
    // to on, so an existing install gets the review step rather than silently
    // starting to draft on a plan nobody saw.
    if from_version < 11 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        obj.entry("draft_pipeline").or_insert(serde_json::json!({
            "plan_gate": "gated",
            "planner_model_id": null,
            "reviewer_model_id": null,
        }));
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(11.into()),
        );
        tracing::info!("migrated config v10 → v11 (added drafting pipeline preferences)");
    }

    // v11 → v12: add the per-call Bedrock runtime dials — the four stream
    // waits and the writer's output ceiling. Existing installs land on the
    // values that were compiled in until now, so nothing changes for anyone
    // who does not go and raise them.
    if from_version < 12 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        if let Some(serde_json::Value::Object(report_authoring)) = obj.get_mut("report_authoring") {
            report_authoring
                .entry("writer_first_frame_timeout_secs")
                .or_insert(serde_json::json!(default_writer_first_frame_timeout_secs()));
            report_authoring
                .entry("writer_idle_timeout_secs")
                .or_insert(serde_json::json!(default_writer_idle_timeout_secs()));
            report_authoring
                .entry("writer_max_output_tokens")
                .or_insert(serde_json::json!(default_writer_max_output_tokens()));
            report_authoring
                .entry("analysis_first_frame_timeout_secs")
                .or_insert(serde_json::json!(
                    default_analysis_first_frame_timeout_secs()
                ));
            report_authoring
                .entry("analysis_idle_timeout_secs")
                .or_insert(serde_json::json!(default_analysis_idle_timeout_secs()));
        }
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(12.into()),
        );
        tracing::info!("migrated config v11 → v12 (added Bedrock runtime limits)");
    }

    // v12 → v13: add the auto-lock settings, off and with no PIN. Turning
    // this on for an existing install would lock a clinician out of records
    // behind a PIN they were never asked to choose.
    if from_version < 13 {
        let obj = json
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("config is not a JSON object"))?;
        obj.entry("security").or_insert(serde_json::json!({
            "auto_lock_enabled": false,
            "auto_lock_timeout_minutes": default_auto_lock_timeout_minutes(),
            "biometric_unlock_enabled": false,
            "pin_hash": null,
        }));
        obj.insert(
            "config_version".to_string(),
            serde_json::Value::Number(13.into()),
        );
        tracing::info!("migrated config v12 → v13 (added auto-lock settings)");
    }

    Ok(json)
}

pub fn save_config(config: &ClariaConfig) -> eyre::Result<()> {
    save_config_at(&config_path()?, config)
}

/// [`save_config`] against an explicit path.
fn save_config_at(path: &Path, config: &ClariaConfig) -> eyre::Result<()> {
    config
        .report_authoring
        .validate()
        .map_err(|error| eyre::eyre!(error))?;
    config
        .model_tuning
        .validate()
        .map_err(|error| eyre::eyre!(error))?;
    let dir = path
        .parent()
        .ok_or_else(|| eyre::eyre!("config path {} has no parent directory", path.display()))?;
    std::fs::create_dir_all(dir)?;

    // Always write the current version, regardless of what was loaded.
    let mut stamped = config.clone();
    stamped.config_version = CURRENT_VERSION;

    let json = serde_json::to_string_pretty(&stamped)?;

    // Write to a temp file then rename for atomicity
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, json.as_bytes())?;

    // Set restrictive permissions on Unix before renaming
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
    }

    std::fs::rename(&tmp_path, path)?;

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
        preferred_model_id: config.preferred_model_id.clone(),
        cost_explorer_enabled: config.cost_explorer_enabled,
        hourly_cost_data: config.hourly_cost_data,
        prompt_caching_enabled: config.prompt_caching_enabled,
        transcription: config.transcription.clone(),
        report_authoring: config.report_authoring.clone(),
        model_tuning: config.model_tuning,
        chat_streaming: config.chat_streaming,
        draft_pipeline: config.draft_pipeline.clone(),
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
