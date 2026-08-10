mod helpers;

use axum::http::{Method, StatusCode};

use helpers::{app, request, request_with_header};

// ── Health & Reset ──────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok() {
    let app = app();
    let r = request(&app, Method::GET, "/mock/health", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body, "ok");
}

#[tokio::test]
async fn reset_clears_all_state() {
    let app = app();
    // Create a bucket
    request(&app, Method::PUT, "/mybucket", "").await;
    let r = request(&app, Method::HEAD, "/mybucket", "").await;
    assert_eq!(r.status, StatusCode::OK);

    // Reset
    let r = request(&app, Method::POST, "/mock/reset", "").await;
    assert_eq!(r.status, StatusCode::OK);

    // Bucket should be gone
    let r = request(&app, Method::HEAD, "/mybucket", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

// ── Scenario loading ────────────────────────────────────────────────

#[tokio::test]
async fn load_fresh_account_scenario() {
    let app = app();
    let r = request(&app, Method::POST, "/mock/scenario/fresh-account", "").await;
    assert_eq!(r.status, StatusCode::OK);

    // Fresh account has bedrock models but no buckets
    let r = request(&app, Method::GET, "/foundation-models", "").await;
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert!(body["modelSummaries"].as_array().unwrap().len() >= 3);
}

#[tokio::test]
async fn load_fully_provisioned_scenario() {
    let app = app();
    let r = request(&app, Method::POST, "/mock/scenario/fully-provisioned", "").await;
    assert_eq!(r.status, StatusCode::OK);

    // Should have a bucket with data
    let r = request(
        &app,
        Method::GET,
        "/185735714230-claria-data?list-type=2",
        "",
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<Key>"));

    // Should have versioning enabled
    let r = request(
        &app,
        Method::GET,
        "/185735714230-claria-data?versioning",
        "",
    )
    .await;
    assert!(r.body.contains("Enabled"));

    // Should have encryption
    let r = request(
        &app,
        Method::GET,
        "/185735714230-claria-data?encryption",
        "",
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("AES256"));

    // CloudTrail should exist
    let target = "com.amazonaws.cloudtrail.v20131101.CloudTrail_20131101.DescribeTrails";
    let r = request_with_header(&app, Method::POST, "/", "x-amz-target", target, "{}").await;
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert!(!body["trailList"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn load_drifted_scenario_has_missing_config() {
    let app = app();
    request(&app, Method::POST, "/mock/scenario/drifted", "").await;

    // Drifted: versioning should be suspended/disabled
    let r = request(
        &app,
        Method::GET,
        "/185735714230-claria-data?versioning",
        "",
    )
    .await;
    assert!(!r.body.contains("<Status>Enabled</Status>"));

    // Drifted: encryption should be missing
    let r = request(
        &app,
        Method::GET,
        "/185735714230-claria-data?encryption",
        "",
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn load_unknown_scenario_returns_400() {
    let app = app();
    let r = request(&app, Method::POST, "/mock/scenario/nonexistent", "").await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

// ── Params helper ───────────────────────────────────────────────────

#[test]
fn params_extract_basic() {
    assert_eq!(
        claria_mock_aws::params::extract("foo=bar&baz=qux", "foo"),
        Some("bar".to_string())
    );
    assert_eq!(
        claria_mock_aws::params::extract("foo=bar&baz=qux", "baz"),
        Some("qux".to_string())
    );
    assert_eq!(claria_mock_aws::params::extract("foo=bar", "missing"), None);
}

#[test]
fn params_extract_url_encoded() {
    assert_eq!(
        claria_mock_aws::params::extract("key=hello%20world", "key"),
        Some("hello world".to_string())
    );
}

// ── XML helpers ─────────────────────────────────────────────────────

#[test]
fn xml_el_produces_element() {
    assert_eq!(claria_mock_aws::xml::el("Foo", "bar"), "<Foo>bar</Foo>");
}

#[test]
fn xml_wrap_ns_produces_element_with_xmlns() {
    let result = claria_mock_aws::xml::wrap_ns("Root", "http://example.com", "<Child/>");
    assert!(result.contains(r#"xmlns="http://example.com""#));
    assert!(result.contains("<Child/>"));
}

#[test]
fn xml_error_format() {
    let err = claria_mock_aws::xml::error_xml("NoSuchKey", "not found");
    assert!(err.contains("<Code>NoSuchKey</Code>"));
    assert!(err.contains("<Message>not found</Message>"));
}

// ── Cost Explorer ───────────────────────────────────────────────────

#[tokio::test]
async fn cost_explorer_empty_by_default() {
    let app = app();
    let target = "com.amazonaws.ce.AWSInsightsIndexService.GetCostAndUsage";
    let r = request_with_header(&app, Method::POST, "/", "x-amz-target", target, "{}").await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["ResultsByTime"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn cost_explorer_after_fully_provisioned() {
    let app = app();
    request(&app, Method::POST, "/mock/scenario/fully-provisioned", "").await;

    let target = "com.amazonaws.ce.AWSInsightsIndexService.GetCostAndUsage";
    let r = request_with_header(&app, Method::POST, "/", "x-amz-target", target, "{}").await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert!(!body["ResultsByTime"].as_array().unwrap().is_empty());
}

// ── Artifact ────────────────────────────────────────────────────────

#[tokio::test]
async fn artifact_no_baa_by_default() {
    let app = app();
    let target = "com.amazonaws.artifact.Artifact.ListCustomerAgreements";
    let r = request_with_header(&app, Method::POST, "/", "x-amz-target", target, "{}").await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["customerAgreements"].as_array().unwrap().len(), 0);
}
