use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::Value;

use crate::{
    params,
    scenarios,
    services::{artifact, bedrock, cloudtrail, cost_explorer, iam, s3, sts, transcribe},
    state::SharedState,
};

pub fn build_router(state: SharedState) -> Router {
    Router::new()
        // Mock control endpoints
        .route("/mock/health", get(health))
        .route("/mock/reset", post(reset))
        .route("/mock/scenario/{name}", post(load_scenario))
        // Catch-all for AWS service requests
        .fallback(dispatch_aws)
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

async fn reset(State(state): State<SharedState>) -> Response {
    let mut st = state.write().await;
    *st = Default::default();
    StatusCode::OK.into_response()
}

async fn load_scenario(
    State(state): State<SharedState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    let mut st = state.write().await;
    *st = Default::default();
    match scenarios::load(&name, &mut st) {
        Ok(()) => (StatusCode::OK, format!("Loaded scenario: {name}")).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("Unknown scenario: {e}")).into_response(),
    }
}

/// Central dispatch: examine headers, path, and query to determine which
/// AWS service the request is targeting.
async fn dispatch_aws(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    State(state): State<SharedState>,
    body: Bytes,
) -> Response {
    let path = uri.path();
    let query = uri.query().unwrap_or("");

    // 1. Check X-Amz-Target header for JSON-protocol services
    if let Some(target) = headers.get("x-amz-target").and_then(|v| v.to_str().ok()) {
        let json_body: Value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap_or(Value::Null)
        };

        // Extract the operation name (after the last `.`)
        let op = target.rsplit('.').next().unwrap_or(target);

        if target.contains("CloudTrail") {
            return cloudtrail::dispatch(op, json_body, state).await;
        }
        if target.starts_with("Transcribe") {
            return transcribe::dispatch(op, json_body, state).await;
        }
        if target.contains("AWSInsightsIndexService") || target.contains("CostExplorer") {
            return cost_explorer::dispatch(op, json_body, state).await;
        }
        if target.contains("artifact") || target.contains("Artifact") {
            return artifact::dispatch(op, json_body, state).await;
        }

        return (
            StatusCode::BAD_REQUEST,
            format!("Unknown X-Amz-Target: {target}"),
        )
            .into_response();
    }

    // 2. Bedrock REST paths
    if path.starts_with("/foundation-models")
        || path.starts_with("/custom-model-agreements")
        || path.starts_with("/inference-profiles")
        || path.starts_with("/model/")
    {
        let json_body: Value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap_or(Value::Null)
        };
        return bedrock::dispatch(&method, path, json_body, state).await;
    }

    // 3. Query-protocol services (IAM / STS): POST with Action= in body
    if method == Method::POST {
        let body_str = String::from_utf8_lossy(&body);

        // Check if body contains Action= (form-encoded)
        if let Some(action) = extract_form_param(&body_str, "Action") {
            // Distinguish IAM from STS by action name
            let sts_actions = [
                "GetCallerIdentity",
                "AssumeRole",
                "GetSessionToken",
                "AssumeRoleWithWebIdentity",
                "AssumeRoleWithSAML",
            ];

            if sts_actions.contains(&action.as_str()) {
                return sts::dispatch(&action, &body_str, state).await;
            }
            return iam::dispatch(&action, &body_str, state).await;
        }

        // Also check query string for Action (some SDKs put it there)
        if let Some(action) = extract_form_param(query, "Action") {
            let sts_actions = [
                "GetCallerIdentity",
                "AssumeRole",
                "GetSessionToken",
            ];
            if sts_actions.contains(&action.as_str()) {
                return sts::dispatch(&action, query, state).await;
            }
            return iam::dispatch(&action, query, state).await;
        }
    }

    // 4. Everything else → S3
    s3::dispatch(method, uri, headers, state, body).await
}

fn extract_form_param(body: &str, key: &str) -> Option<String> {
    params::extract(body, key)
}
