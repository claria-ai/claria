use std::sync::Arc;

use aws_sdk_s3::Client as S3Client;
use tokio::sync::Mutex;

use claria_search::index::LoadedIndex;

/// Shared application state, injected into all route handlers via Axum state.
#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub s3: S3Client,
    pub bucket: String,
    pub index: Arc<Mutex<LoadedIndex>>,
    pub cognito_user_pool_id: String,
    pub cognito_region: String,
}
