use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::Value;

use crate::{
    faults, params, scenarios,
    services::{artifact, bedrock, cloudtrail, cost_explorer, iam, s3, sts, transcribe},
    state::SharedState,
};

/// Cap on request body size for the mock S3 PutObject path. Real S3 single-PUT
/// caps at 5 GB; we just need to be comfortably bigger than any audio fixture
/// or transcript JSON a test might upload. 64 MB is generous and keeps a clear
/// boundary against runaway requests.
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

pub fn build_router(state: SharedState) -> Router {
    Router::new()
        // Mock control endpoints
        .route("/mock/health", get(health))
        .route("/mock/reset", post(reset))
        .route("/mock/scenario/{name}", post(load_scenario))
        // Catch-all for AWS service requests
        .fallback(dispatch_aws)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
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

        if let Some(refusal) = faults::injected(op, faults::Wire::Json, &state).await {
            return refusal;
        }

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

    // 2. Artifact REST path. `ListCustomerAgreements` is REST-JSON, not one of
    // the `x-amz-target` protocols, so without this it falls through to S3 and
    // the BAA precondition reads as a service error no scan can explain.
    if path.starts_with("/v1/customer-agreement") {
        if let Some(refusal) =
            faults::injected("ListCustomerAgreements", faults::Wire::Json, &state).await
        {
            return refusal;
        }
        return artifact::dispatch("ListCustomerAgreements", Value::Null, state).await;
    }

    // 3. Bedrock REST paths
    if path.starts_with("/foundation-models")
        || path.starts_with("/foundation-model-availability/")
        || path.starts_with("/list-foundation-model-agreement-offers/")
        || path.starts_with("/create-foundation-model-agreement")
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

    // 4. Query-protocol services (IAM / STS): POST with Action= in body
    if method == Method::POST {
        let body_str = String::from_utf8_lossy(&body);

        // Check if body contains Action= (form-encoded)
        if let Some(action) = extract_form_param(&body_str, "Action") {
            if let Some(refusal) = faults::injected(&action, faults::Wire::Query, &state).await {
                return refusal;
            }

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
            if let Some(refusal) = faults::injected(&action, faults::Wire::Query, &state).await {
                return refusal;
            }

            let sts_actions = ["GetCallerIdentity", "AssumeRole", "GetSessionToken"];
            if sts_actions.contains(&action.as_str()) {
                return sts::dispatch(&action, query, state).await;
            }
            return iam::dispatch(&action, query, state).await;
        }
    }

    // 5. Everything else → S3
    s3::dispatch(method, uri, headers, state, body).await
}

fn extract_form_param(body: &str, key: &str) -> Option<String> {
    params::extract(body, key)
}
