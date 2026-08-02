use std::{collections::HashMap, sync::Arc};

use tokio::sync::Mutex;

use claria_desktop::{
    config::{ClariaConfig, CredentialSource},
    record_cache::RecordCache,
};

/// An `SdkConfig` cached with the inputs it was built from. The SDK's pooled
/// HTTP connector lives inside the `SdkConfig`, so reusing it across commands
/// keeps connections warm instead of paying DNS/TCP/TLS setup per call.
pub struct CachedSdkConfig {
    pub region: String,
    pub credentials: CredentialSource,
    pub sdk_config: aws_config::SdkConfig,
}

pub(crate) struct PendingReportTemplate {
    pub(crate) client_id: uuid::Uuid,
    pub(crate) imported: claria_docx::ImportedTemplate,
}

pub struct DesktopState {
    pub config: Arc<Mutex<Option<ClariaConfig>>>,
    pub sdk_config: Arc<Mutex<Option<CachedSdkConfig>>>,
    pub whisper: Arc<std::sync::Mutex<Option<claria_whisper::WhisperModel>>>,
    pub record_cache: Arc<RecordCache>,
    /// Parsed DOCX previews waiting for explicit user acceptance. Source bytes,
    /// filenames, and local paths are never retained.
    pub(crate) pending_report_templates: Arc<Mutex<HashMap<uuid::Uuid, PendingReportTemplate>>>,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self {
            config: Arc::new(Mutex::new(None)),
            sdk_config: Arc::new(Mutex::new(None)),
            whisper: Arc::new(std::sync::Mutex::new(None)),
            record_cache: Arc::new(RecordCache::new()),
            pending_report_templates: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
