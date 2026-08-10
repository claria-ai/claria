mod helpers;

use axum::http::{Method, StatusCode};

use helpers::{app, request_with_header};

async fn ct_request(app: &axum::Router, op: &str, json: &str) -> helpers::TestResponse {
    let target = format!("com.amazonaws.cloudtrail.v20131101.CloudTrail_20131101.{op}");
    let body = json.to_string();
    request_with_header(app, Method::POST, "/", "x-amz-target", &target, body).await
}

#[tokio::test]
async fn create_and_get_trail() {
    let app = app();
    let r = ct_request(
        &app,
        "CreateTrail",
        r#"{
        "Name": "my-trail",
        "S3BucketName": "my-bucket",
        "IsMultiRegionTrail": true
    }"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["Name"], "my-trail");
    assert_eq!(body["S3BucketName"], "my-bucket");
    assert!(body["TrailARN"].as_str().unwrap().contains("my-trail"));

    let r = ct_request(&app, "GetTrail", r#"{"Name": "my-trail"}"#).await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["Trail"]["Name"], "my-trail");
}

#[tokio::test]
async fn get_nonexistent_trail_returns_404() {
    let app = app();
    let r = ct_request(&app, "GetTrail", r#"{"Name": "nope"}"#).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn start_and_stop_logging() {
    let app = app();
    ct_request(
        &app,
        "CreateTrail",
        r#"{"Name": "log-trail", "S3BucketName": "bucket"}"#,
    )
    .await;

    ct_request(&app, "StartLogging", r#"{"Name": "log-trail"}"#).await;

    let r = ct_request(&app, "GetTrailStatus", r#"{"Name": "log-trail"}"#).await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["IsLogging"], true);

    ct_request(&app, "StopLogging", r#"{"Name": "log-trail"}"#).await;

    let r = ct_request(&app, "GetTrailStatus", r#"{"Name": "log-trail"}"#).await;
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["IsLogging"], false);
}

#[tokio::test]
async fn describe_trails_lists_all() {
    let app = app();
    ct_request(
        &app,
        "CreateTrail",
        r#"{"Name": "trail-a", "S3BucketName": "b"}"#,
    )
    .await;
    ct_request(
        &app,
        "CreateTrail",
        r#"{"Name": "trail-b", "S3BucketName": "b"}"#,
    )
    .await;

    let r = ct_request(&app, "DescribeTrails", "{}").await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    let trails = body["trailList"].as_array().unwrap();
    assert_eq!(trails.len(), 2);
}

#[tokio::test]
async fn delete_trail() {
    let app = app();
    ct_request(
        &app,
        "CreateTrail",
        r#"{"Name": "del-trail", "S3BucketName": "b"}"#,
    )
    .await;

    let r = ct_request(&app, "DeleteTrail", r#"{"Name": "del-trail"}"#).await;
    assert_eq!(r.status, StatusCode::OK);

    let r = ct_request(&app, "GetTrail", r#"{"Name": "del-trail"}"#).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}
