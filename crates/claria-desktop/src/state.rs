use std::sync::Arc;

use tokio::sync::Mutex;

use claria_desktop::config::ClariaConfig;

pub struct DesktopState {
    pub config: Arc<Mutex<Option<ClariaConfig>>>,
    pub whisper: Arc<std::sync::Mutex<Option<claria_whisper::WhisperModel>>>,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self {
            config: Arc::new(Mutex::new(None)),
            whisper: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}
