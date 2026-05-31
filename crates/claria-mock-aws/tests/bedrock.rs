mod helpers;

use axum::http::{Method, StatusCode};

use helpers::{app, request, request_with_header};

const OPUS: &str = "anthropic.claude-opus-4-6-20260301-v1:0";

/// Build a router with a preloaded scenario.
async fn app_with(scenario: &str) -> axum::Router {
    let state = claria_mock_aws::state::new_shared_state();
    {
        let mut st = state.write().await;
        claria_mock_aws::scenarios::load(scenario, &mut st).unwrap();
    }
    claria_mock_aws::router::build_router(state)
}

fn post_json(uri: &'static str, body: &'static str) -> (Method, &'static str, &'static str, &'static str, &'static str) {
    (Method::POST, uri, "content-type", "application/json", body)
}

#[tokio::test]
async fn list_foundation_models_empty_by_default() {
    let app = app();
    let r = request(&app, Method::GET, "/foundation-models", "").await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["modelSummaries"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_foundation_models_after_scenario() {
    let app = app_with("fresh-account").await;
    let r = request(&app, Method::GET, "/foundation-models", "").await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    let models = body["modelSummaries"].as_array().unwrap();
    assert!(models.len() >= 3);
    assert!(models.iter().any(|m| m["providerName"] == "Anthropic"));
}

#[tokio::test]
async fn availability_returns_four_axes() {
    let app = app_with("fresh-account").await;
    let r = request(
        &app,
        Method::GET,
        &format!("/foundation-model-availability/{OPUS}"),
        "",
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    // fresh-account: an offer is available but not yet executed.
    assert_eq!(body["agreementAvailability"]["status"], "AVAILABLE");
    assert_eq!(body["entitlementAvailability"], "NOT_AVAILABLE");
    assert_eq!(body["authorizationStatus"], "AUTHORIZED");
    assert_eq!(body["regionAvailability"], "AVAILABLE");
}

#[tokio::test]
async fn offers_include_terms_and_pricing() {
    let app = app_with("agreements-available").await;
    let r = request(
        &app,
        Method::GET,
        &format!("/list-foundation-model-agreement-offers/{OPUS}"),
        "",
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    let offer = &body["offers"][0];
    assert!(offer["offerToken"].as_str().unwrap().contains(OPUS));
    assert!(offer["termDetails"]["legalTerm"]["url"].as_str().unwrap().starts_with("https://"));
    let rate_card = offer["termDetails"]["usageBasedPricingTerm"]["rateCard"]
        .as_array()
        .unwrap();
    assert_eq!(rate_card.len(), 2);
}

#[tokio::test]
async fn use_case_form_absent_until_submitted() {
    let app = app_with("ftu-required").await;
    let r = request(&app, Method::GET, "/use-case-for-model-access", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.body.contains("have not been submitted"));
}

#[tokio::test]
async fn create_agreement_blocked_until_use_case_form_submitted() {
    let app = app_with("ftu-required").await;
    let (m, u, hn, hv, b) = post_json(
        "/create-foundation-model-agreement",
        r#"{"modelId":"anthropic.claude-opus-4-6-20260301-v1:0","offerToken":"t"}"#,
    );
    let r = request_with_header(&app, m, u, hn, hv, b).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.body.contains("FTUFormNotFilled"));
}

#[tokio::test]
async fn put_use_case_form_then_get_round_trips() {
    let app = app_with("ftu-required").await;
    let (m, u, hn, hv, b) = post_json(
        "/use-case-for-model-access",
        r#"{"formData":"eyJjb21wYW55TmFtZSI6IkFjbWUifQ=="}"#,
    );
    let r = request_with_header(&app, m, u, hn, hv, b).await;
    assert_eq!(r.status, StatusCode::CREATED);

    let r = request(&app, Method::GET, "/use-case-for-model-access", "").await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["formData"], "eyJjb21wYW55TmFtZSI6IkFjbWUifQ==");
}

#[tokio::test]
async fn execute_then_promote_drives_pending_to_executed() {
    let app = app_with("agreements-available").await;

    // Execute → PENDING, returns 202.
    let (m, u, hn, hv, b) = post_json(
        "/create-foundation-model-agreement",
        r#"{"modelId":"anthropic.claude-opus-4-6-20260301-v1:0","offerToken":"t"}"#,
    );
    let r = request_with_header(&app, m, u, hn, hv, b).await;
    assert_eq!(r.status, StatusCode::ACCEPTED);

    let r = request(&app, Method::GET, &format!("/foundation-model-availability/{OPUS}"), "").await;
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["agreementAvailability"]["status"], "PENDING");
    assert_eq!(body["entitlementAvailability"], "NOT_AVAILABLE");

    // Promote (simulates the async marketplace subscription completing).
    let r = request(&app, Method::POST, &format!("/mock/bedrock/promote/{OPUS}"), "").await;
    assert_eq!(r.status, StatusCode::OK);

    let r = request(&app, Method::GET, &format!("/foundation-model-availability/{OPUS}"), "").await;
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["entitlementAvailability"], "AVAILABLE");
}

#[tokio::test]
async fn fully_provisioned_models_are_executed() {
    let app = app_with("fully-provisioned").await;
    let r = request(&app, Method::GET, &format!("/foundation-model-availability/{OPUS}"), "").await;
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["entitlementAvailability"], "AVAILABLE");

    // FTU already submitted.
    let r = request(&app, Method::GET, "/use-case-for-model-access", "").await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn converse_returns_canned_response() {
    let app = app();
    let body = serde_json::json!({
        "messages": [{"role": "user", "content": [{"text": "Hello"}]}],
        "modelId": "test"
    });
    let r = request_with_header(
        &app, Method::POST, "/model/test/converse",
        "content-type", "application/json",
        serde_json::to_string(&body).unwrap(),
    ).await;
    assert_eq!(r.status, StatusCode::OK);
    let resp: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert!(!resp["output"]["message"]["content"][0]["text"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn list_inference_profiles_empty_by_default() {
    let app = app();
    let r = request(&app, Method::GET, "/inference-profiles", "").await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["inferenceProfileSummaries"].as_array().unwrap().len(), 0);
}
