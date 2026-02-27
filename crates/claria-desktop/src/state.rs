use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AwsConfig {
    pub region: String,
    pub bucket: String,
}

pub struct DesktopState {
    pub config: Arc<Mutex<Option<AwsConfig>>>,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self {
            config: Arc::new(Mutex::new(None)),
        }
    }
}
