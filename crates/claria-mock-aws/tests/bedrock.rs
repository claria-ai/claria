mod helpers;

use axum::http::{Method, StatusCode};

use helpers::{app, request, request_with_header};

fn report_tool_config() -> serde_json::Value {
    serde_json::json!({
        "tools": [
            {"toolSpec": {"name": "list_record_files", "inputSchema": {"json": {
                "type": "object", "properties": {}, "additionalProperties": false
            }}}},
            {"toolSpec": {"name": "read_record_file", "inputSchema": {"json": {
                "type": "object", "required": ["filename"], "properties": {"filename": {"type": "string"}}, "additionalProperties": false
            }}}},
            {"toolSpec": {"name": "read_report_section", "inputSchema": {"json": {
                "type": "object", "required": ["section_id"], "properties": {"section_id": {"type": "string"}}, "additionalProperties": false
            }}}},
            {"toolSpec": {"name": "propose_report_changes", "inputSchema": {"json": {
                "type": "object", "required": ["summary", "operations"], "properties": {"summary": {"type": "string"}, "operations": {"type": "array"}}, "additionalProperties": false
            }}}}
        ]
    })
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

    let r = request(
        &app,
        Method::GET,
        "/foundation-models/anthropic.claude-opus-4-6-20260301-v1:0/availability",
        "",
    )
    .await;
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
        &app,
        Method::POST,
        "/custom-model-agreements",
        "content-type",
        "application/json",
        r#"{"modelId": "anthropic.claude-opus-4-6-20260301-v1:0"}"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);

    let r = request(
        &app,
        Method::GET,
        "/foundation-models/anthropic.claude-opus-4-6-20260301-v1:0/availability",
        "",
    )
    .await;
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
        &app,
        Method::POST,
        "/model/test/converse",
        "content-type",
        "application/json",
        serde_json::to_string(&body).unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let resp: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert!(
        !resp["output"]["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn tool_configured_converse_uses_fifo_scripts_without_affecting_chat() {
    let state = claria_mock_aws::state::new_shared_state();
    {
        let mut st = state.write().await;
        st.bedrock_tool_responses.push(
            claria_mock_aws::state::ScriptedBedrockResponse::success(
                serde_json::json!({
                    "output": {"message": {"role": "assistant", "content": [{"text": "scripted report"}]}},
                    "stopReason": "end_turn"
                }),
            ),
        );
    }
    let app = claria_mock_aws::router::build_router(state.clone());

    let ordinary = request_with_header(
        &app,
        Method::POST,
        "/model/chat-model/converse",
        "content-type",
        "application/json",
        r#"{"messages":[{"role":"user","content":[{"text":"hello"}]}]}"#,
    )
    .await;
    assert_eq!(ordinary.status, StatusCode::OK);
    assert!(!ordinary.body.contains("scripted report"));
    assert_eq!(state.read().await.bedrock_tool_responses.len(), 1);

    let report_body = serde_json::json!({
        "messages": [{"role": "user", "content": [{"text": "draft"}]}],
        "toolConfig": report_tool_config()
    });
    let report = request_with_header(
        &app,
        Method::POST,
        "/model/us.anthropic%2Freport/converse",
        "content-type",
        "application/json",
        report_body.to_string(),
    )
    .await;
    assert_eq!(report.status, StatusCode::OK);
    assert!(report.body.contains("scripted report"));

    let st = state.read().await;
    assert!(st.bedrock_tool_responses.is_empty());
    assert_eq!(st.bedrock_tool_requests.len(), 1);
    assert_eq!(st.bedrock_tool_model_ids, vec!["us.anthropic/report"]);
}

#[tokio::test]
async fn tool_configured_converse_can_script_service_errors() {
    let state = claria_mock_aws::state::new_shared_state();
    state.write().await.bedrock_tool_responses.push(
        claria_mock_aws::state::ScriptedBedrockResponse::error(
            400,
            serde_json::json!({
                "__type": "ValidationException",
                "message": "This model does not support tool use"
            }),
        ),
    );
    let app = claria_mock_aws::router::build_router(state);
    let body = serde_json::json!({
        "messages": [{"role": "user", "content": [{"text": "draft"}]}],
        "toolConfig": report_tool_config()
    });
    let response = request_with_header(
        &app,
        Method::POST,
        "/model/test/converse",
        "content-type",
        "application/json",
        body.to_string(),
    )
    .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert!(response.body.contains("ValidationException"));
}

#[tokio::test]
async fn report_protocol_validation_rejects_bad_method_schema_and_correlation() {
    let app = app();
    let valid = serde_json::json!({
        "messages": [{"role": "user", "content": [{"text": "draft"}]}],
        "toolConfig": report_tool_config()
    });
    let wrong_method = request_with_header(
        &app,
        Method::GET,
        "/model/test/converse",
        "content-type",
        "application/json",
        valid.to_string(),
    )
    .await;
    assert_eq!(wrong_method.status, StatusCode::METHOD_NOT_ALLOWED);

    let bad_schema = serde_json::json!({
        "messages": [{"role": "user", "content": [{"text": "draft"}]}],
        "toolConfig": {"tools": []}
    });
    let response = request_with_header(
        &app,
        Method::POST,
        "/model/test/converse",
        "content-type",
        "application/json",
        bad_schema.to_string(),
    )
    .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert!(response.body.contains("ValidationException"));

    let bad_correlation = serde_json::json!({
        "messages": [
            {"role": "user", "content": [{"text": "draft"}]},
            {"role": "assistant", "content": [{"toolUse": {"toolUseId": "one", "name": "list_record_files", "input": {}}}]},
            {"role": "user", "content": [{"toolResult": {"toolUseId": "different", "status": "error", "content": [{"json": {}}]}}]}
        ],
        "toolConfig": report_tool_config()
    });
    let response = request_with_header(
        &app,
        Method::POST,
        "/model/test/converse",
        "content-type",
        "application/json",
        bad_correlation.to_string(),
    )
    .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn list_inference_profiles_empty_by_default() {
    let app = app();
    let r = request(&app, Method::GET, "/inference-profiles", "").await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(
        body["inferenceProfileSummaries"].as_array().unwrap().len(),
        0
    );
}
