use std::sync::Arc;

use tokio::sync::Mutex;

use crate::config::ClariaConfig;

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
