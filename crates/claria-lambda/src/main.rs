use std::env;
use std::path::Path;
use std::sync::Arc;

use axum::middleware as axum_mw;
use axum::routing::{delete, get, post, put};
use axum::Router;
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::EnvFilter;

mod error;
mod middleware;
mod routes;
mod state;

use state::AppState;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Structured JSON logging for CloudWatch
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .json()
        .init();

    let bucket = env::var("CLARIA_BUCKET").unwrap_or_else(|_| "claria".to_string());
    let cognito_user_pool_id =
        env::var("COGNITO_USER_POOL_ID").unwrap_or_else(|_| "us-east-1_placeholder".to_string());
    let cognito_region =
        env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());

    let s3 = claria_storage::client::build_client().await;

    // Try to download the Tantivy index; create empty if not found.
    let index_dir = Path::new("/tmp/tantivy");
    let loaded_index =
        match claria_search::index::download_index(&s3, &bucket, index_dir).await {
            Ok(idx) => idx,
            Err(claria_search::error::SearchError::IndexNotFound) => {
                tracing::info!("no existing index found, creating empty index");
                std::fs::create_dir_all(index_dir)?;
                let index = claria_search::index::create_empty_index(index_dir)?;
                claria_search::index::LoadedIndex {
                    index,
                    index_dir: index_dir.to_path_buf(),
                    etag: String::new(),
                }
            }
            Err(e) => return Err(e.into()),
        };

    let state = AppState {
        s3,
        bucket,
        index: Arc::new(Mutex::new(loaded_index)),
        cognito_user_pool_id,
        cognito_region,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        // Health (no auth)
        .route("/health", get(routes::health::health_check))
        // Instruments (no auth — public schema data)
        .route("/instruments", get(routes::instruments::list_instruments))
        .route(
            "/instruments/{id}",
            get(routes::instruments::get_instrument_detail),
        )
        // Protected routes
        .route("/assessments", get(routes::assessments::list_assessments))
        .route("/assessments", post(routes::assessments::create_assessment))
        .route(
            "/assessments/{id}",
            get(routes::assessments::get_assessment),
        )
        .route(
            "/assessments/{id}",
            put(routes::assessments::update_assessment),
        )
        .route(
            "/assessments/{id}",
            delete(routes::assessments::delete_assessment),
        )
        .route("/snippets", get(routes::snippets::list_snippets))
        .route("/snippets", post(routes::snippets::create_snippet))
        .route("/snippets/{id}", get(routes::snippets::get_snippet))
        .route("/snippets/{id}", put(routes::snippets::update_snippet))
        .route("/snippets/{id}", delete(routes::snippets::delete_snippet))
        .route("/goals", get(routes::goals::list_goals))
        .route("/goals", post(routes::goals::create_goal))
        .route("/goals/{id}", get(routes::goals::get_goal))
        .route("/goals/{id}", put(routes::goals::update_goal))
        .route("/goals/{id}", delete(routes::goals::delete_goal))
        .route("/templates", get(routes::templates::list_templates))
        .route("/templates", post(routes::templates::create_template))
        .route("/templates/{id}", get(routes::templates::get_template))
        .route("/templates/{id}", put(routes::templates::update_template))
        .route(
            "/templates/{id}",
            delete(routes::templates::delete_template),
        )
        .route("/reports", get(routes::reports::list_reports))
        .route("/reports/{id}", get(routes::reports::get_report))
        .route("/reports/{id}/export", post(routes::reports::export_report))
        .route(
            "/transactions",
            get(routes::transactions::list_transactions),
        )
        .route(
            "/transactions/{id}",
            get(routes::transactions::get_transaction),
        )
        .route("/anonymize", post(routes::anonymize::anonymize))
        .route("/cost/estimate", post(routes::cost::estimate_cost))
        .layer(axum_mw::from_fn(middleware::audit::audit_log))
        .layer(cors)
        .with_state(state);

    lambda_http::run(app).await.map_err(|e| eyre::eyre!(e))
}
