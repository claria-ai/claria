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
    pub(crate) writer_template_id: uuid::Uuid,
    pub(crate) writer_template_name: String,
    pub(crate) source_docx: Vec<u8>,
    pub(crate) imported: claria_docx::ImportedTemplate,
}

pub struct DesktopState {
    pub config: Arc<Mutex<Option<ClariaConfig>>>,
    pub sdk_config: Arc<Mutex<Option<CachedSdkConfig>>>,
    pub local_transcriber: Arc<std::sync::Mutex<claria_transcribe::LocalTranscriber>>,
    pub record_cache: Arc<RecordCache>,
    /// Parsed managed-template candidates. Validated source bytes stay only
    /// long enough to write the immutable formatting snapshot; local paths are
    /// never retained.
    pub(crate) pending_report_templates: Arc<Mutex<HashMap<uuid::Uuid, PendingReportTemplate>>>,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self {
            config: Arc::new(Mutex::new(None)),
            sdk_config: Arc::new(Mutex::new(None)),
            local_transcriber: Arc::new(std::sync::Mutex::new(
                claria_transcribe::LocalTranscriber::default(),
            )),
            record_cache: Arc::new(RecordCache::new()),
            pending_report_templates: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
