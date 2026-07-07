use std::sync::Arc;

use tokio::sync::Mutex;

use claria_desktop::config::{ClariaConfig, CredentialSource};
use claria_desktop::record_cache::RecordCache;

/// An `SdkConfig` cached with the inputs it was built from. The SDK's pooled
/// HTTP connector lives inside the `SdkConfig`, so reusing it across commands
/// keeps connections warm instead of paying DNS/TCP/TLS setup per call.
pub struct CachedSdkConfig {
    pub region: String,
    pub credentials: CredentialSource,
    pub sdk_config: aws_config::SdkConfig,
}

pub struct DesktopState {
    pub config: Arc<Mutex<Option<ClariaConfig>>>,
    pub sdk_config: Arc<Mutex<Option<CachedSdkConfig>>>,
    pub whisper: Arc<std::sync::Mutex<Option<claria_whisper::WhisperModel>>>,
    pub record_cache: Arc<RecordCache>,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self {
            config: Arc::new(Mutex::new(None)),
            sdk_config: Arc::new(Mutex::new(None)),
            whisper: Arc::new(std::sync::Mutex::new(None)),
            record_cache: Arc::new(RecordCache::new()),
        }
    }
}
