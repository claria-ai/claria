use std::sync::Arc;

use tokio::sync::Mutex;

use claria_desktop::config::ClariaConfig;

pub struct DesktopState {
    pub config: Arc<Mutex<Option<ClariaConfig>>>,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self {
            config: Arc::new(Mutex::new(None)),
        }
    }
}
