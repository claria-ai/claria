use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use crate::state::{SharedState, Trail};

/// Dispatch CloudTrail JSON-protocol requests.
/// `target_suffix` is the operation name after the last `.` in `X-Amz-Target`.
pub async fn dispatch(target_suffix: &str, body: Value, state: SharedState) -> Response {
    match target_suffix {
        "CreateTrail" => create_trail(body, state).await,
        "GetTrail" => get_trail(body, state).await,
        "DescribeTrails" => describe_trails(state).await,
        "DeleteTrail" => delete_trail(body, state).await,
        "StartLogging" => start_logging(body, state).await,
        "StopLogging" => stop_logging(body, state).await,
        "GetTrailStatus" => get_trail_status(body, state).await,
        _ => (
            StatusCode::BAD_REQUEST,
            json!({"__type": "InvalidAction", "message": format!("Unknown CloudTrail action: {target_suffix}")}).to_string(),
        ).into_response(),
    }
}

fn json_response(value: Value) -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/x-amz-json-1.1")],
        value.to_string(),
    )
        .into_response()
}

async fn create_trail(body: Value, state: SharedState) -> Response {
    let name = body["Name"].as_str().unwrap_or("").to_string();
    let s3_bucket = body["S3BucketName"].as_str().unwrap_or("").to_string();
    let s3_prefix = body["S3KeyPrefix"].as_str().map(|s| s.to_string());
    let multi_region = body["IsMultiRegionTrail"].as_bool().unwrap_or(false);

    let mut st = state.write().await;
    let account = st.caller_identity.account.clone();
    let arn = format!("arn:aws:cloudtrail:us-east-1:{account}:trail/{name}");

    let trail = Trail {
        name: name.clone(),
        trail_arn: arn.clone(),
        s3_bucket_name: s3_bucket.clone(),
        s3_key_prefix: s3_prefix.clone(),
        is_multi_region: multi_region,
    };

    st.trails.insert(name.clone(), trail);
    st.trail_logging.insert(name.clone(), false);

    json_response(json!({
        "Name": name,
        "TrailARN": arn,
        "S3BucketName": s3_bucket,
        "S3KeyPrefix": s3_prefix,
        "IsMultiRegionTrail": multi_region,
    }))
}

async fn get_trail(body: Value, state: SharedState) -> Response {
    let name = body["Name"].as_str().unwrap_or("");
    let st = state.read().await;

    match st.trails.get(name) {
        Some(trail) => json_response(json!({
            "Trail": {
                "Name": trail.name,
                "TrailARN": trail.trail_arn,
                "S3BucketName": trail.s3_bucket_name,
                "S3KeyPrefix": trail.s3_key_prefix,
                "IsMultiRegionTrail": trail.is_multi_region,
            }
        })),
        None => (
            StatusCode::NOT_FOUND,
            json!({"__type": "TrailNotFoundException", "message": "Trail not found"}).to_string(),
        )
            .into_response(),
    }
}

async fn describe_trails(state: SharedState) -> Response {
    let st = state.read().await;
    let trails: Vec<Value> = st
        .trails
        .values()
        .map(|t| {
            json!({
                "Name": t.name,
                "TrailARN": t.trail_arn,
                "S3BucketName": t.s3_bucket_name,
                "S3KeyPrefix": t.s3_key_prefix,
                "IsMultiRegionTrail": t.is_multi_region,
            })
        })
        .collect();

    json_response(json!({ "trailList": trails }))
}

async fn delete_trail(body: Value, state: SharedState) -> Response {
    let name = body["Name"].as_str().unwrap_or("");
    let mut st = state.write().await;
    st.trails.remove(name);
    st.trail_logging.remove(name);
    json_response(json!({}))
}

async fn start_logging(body: Value, state: SharedState) -> Response {
    let name = body["Name"].as_str().unwrap_or("");
    let mut st = state.write().await;
    st.trail_logging.insert(name.to_string(), true);
    json_response(json!({}))
}

async fn stop_logging(body: Value, state: SharedState) -> Response {
    let name = body["Name"].as_str().unwrap_or("");
    let mut st = state.write().await;
    st.trail_logging.insert(name.to_string(), false);
    json_response(json!({}))
}

async fn get_trail_status(body: Value, state: SharedState) -> Response {
    let name = body["Name"].as_str().unwrap_or("");
    let st = state.read().await;
    let logging = st.trail_logging.get(name).copied().unwrap_or(false);
    json_response(json!({ "IsLogging": logging }))
}
