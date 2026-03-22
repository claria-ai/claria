mod helpers;

use axum::http::{Method, StatusCode};

use helpers::{app, request, request_with_header};

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
    let app = {
        let state = claria_mock_aws::state::new_shared_state();
        {
            let mut st = state.write().await;
            claria_mock_aws::scenarios::load("fresh-account", &mut st).unwrap();
        }
        claria_mock_aws::router::build_router(state)
    };

    let r = request(&app, Method::GET, "/foundation-models", "").await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    let models = body["modelSummaries"].as_array().unwrap();
    assert!(models.len() >= 3);
    assert!(models.iter().any(|m| m["providerName"] == "Anthropic"));
}

#[tokio::test]
async fn model_availability_not_agreed_by_default() {
    let app = {
        let state = claria_mock_aws::state::new_shared_state();
        {
            let mut st = state.write().await;
            claria_mock_aws::scenarios::load("fresh-account", &mut st).unwrap();
        }
        claria_mock_aws::router::build_router(state)
    };

    let r = request(&app, Method::GET, "/foundation-models/anthropic.claude-opus-4-6-20260301-v1:0/availability", "").await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["agreementAvailability"]["status"], "NOT_AGREED");
}

#[tokio::test]
async fn create_agreement_makes_model_available() {
    let app = {
        let state = claria_mock_aws::state::new_shared_state();
        {
            let mut st = state.write().await;
            claria_mock_aws::scenarios::load("fresh-account", &mut st).unwrap();
        }
        claria_mock_aws::router::build_router(state)
    };

    let r = request_with_header(
        &app, Method::POST, "/custom-model-agreements",
        "content-type", "application/json",
        r#"{"modelId": "anthropic.claude-opus-4-6-20260301-v1:0"}"#,
    ).await;
    assert_eq!(r.status, StatusCode::OK);

    let r = request(&app, Method::GET, "/foundation-models/anthropic.claude-opus-4-6-20260301-v1:0/availability", "").await;
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["agreementAvailability"]["status"], "AVAILABLE");
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
