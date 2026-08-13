//! Output-token limits and input budgets on the plain Converse flows.
//!
//! Every call must set `max_tokens` and branch on `stop_reason`: truncation
//! and context overflow are typed errors, never silently persisted output.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_sdk_bedrockruntime::types::DocumentFormat;
use claria_bedrock::{
    chat::{CHAT_MAX_OUTPUT_TOKENS, CacheStrategy, ChatMessage, ChatRole, chat_converse_stream},
    error::BedrockError,
    extract::{DEFAULT_EXTRACTION_PROMPT, EXTRACT_MAX_OUTPUT_TOKENS, extract_document_text},
};
use claria_mock_aws::{
    state::{FoundationModel, ModelLifecycle, ScriptedBedrockResponse},
    testing::MockServer,
};

const MODEL_ID: &str = "us.anthropic.claude-sonnet-test";

fn sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let credentials = Credentials::new("test", "test", None, None, "test");
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

fn user_messages(text: &str) -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: ChatRole::User,
        content: text.to_string(),
    }]
}

async fn add_haiku(server: &MockServer) {
    server
        .state
        .write()
        .await
        .foundation_models
        .push(FoundationModel {
            model_id: "anthropic.claude-haiku-4-5-20251001-v1:0".to_string(),
            model_name: "Claude Haiku 4.5".to_string(),
            provider_name: "Anthropic".to_string(),
            model_lifecycle: ModelLifecycle {
                status: "ACTIVE".to_string(),
            },
        });
}

#[tokio::test]
async fn chat_sets_max_tokens_and_skips_counting_far_from_budget() {
    let server = MockServer::spawn().await;
    let outcome = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        CacheStrategy::disabled(),
        claria_bedrock::converse::ModelTuning::default(),
        |_| {},
    )
    .await
    .expect("chat");
    assert!(!outcome.text.is_empty());
    assert_eq!(outcome.usage.expect("usage").input_tokens, 150);

    let state = server.state.read().await;
    assert_eq!(state.bedrock_text_requests.len(), 1);
    let request = &state.bedrock_text_requests[0];
    assert_eq!(
        request["inferenceConfig"]["maxTokens"],
        CHAT_MAX_OUTPUT_TOKENS
    );
    assert!(request["inferenceConfig"].get("temperature").is_none());
    // Far below the budget: no CountTokens pre-flight was made.
    assert!(state.bedrock_count_token_requests.is_empty());
}

#[tokio::test]
async fn chat_truncation_is_a_typed_error() {
    let server = MockServer::spawn().await;
    server
        .state
        .write()
        .await
        .bedrock_text_responses
        .push(ScriptedBedrockResponse::success(serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "partial"}]}},
            "stopReason": "max_tokens",
            "usage": {"inputTokens": 10, "outputTokens": 8192}
        })));

    let error = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        CacheStrategy::disabled(),
        claria_bedrock::converse::ModelTuning::default(),
        |_| {},
    )
    .await
    .expect_err("truncation");
    assert!(matches!(
        error,
        BedrockError::ResponseTruncated {
            max_output_tokens
        } if max_output_tokens == CHAT_MAX_OUTPUT_TOKENS
    ));
}

#[tokio::test]
async fn chat_context_overflow_maps_to_friendly_error() {
    let server = MockServer::spawn().await;
    server
        .state
        .write()
        .await
        .bedrock_text_responses
        .push(ScriptedBedrockResponse::success(serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": ""}]}},
            "stopReason": "model_context_window_exceeded"
        })));

    let error = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        CacheStrategy::disabled(),
        claria_bedrock::converse::ModelTuning::default(),
        |_| {},
    )
    .await
    .expect_err("overflow");
    assert!(matches!(error, BedrockError::ContextWindowExceeded));
    assert!(error.to_string().contains("Shorten the conversation"));
}

#[tokio::test]
async fn chat_near_budget_verifies_and_rejects_before_spending() {
    let server = MockServer::spawn().await;
    add_haiku(&server).await;
    // A verified count above the 191,808-token budget (200k window − 8k
    // output reserve) must fail before any billed Converse call.
    server.state.write().await.bedrock_count_tokens_override = Some(200_000);

    // ~700k chars ≈ 175k estimated tokens: within 10% of the budget, so the
    // real CountTokens verification runs.
    let big_prompt = "x".repeat(700_000);
    let error = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        &big_prompt,
        &user_messages("Hello"),
        CacheStrategy::disabled(),
        claria_bedrock::converse::ModelTuning::default(),
        |_| {},
    )
    .await
    .expect_err("budget");
    assert!(matches!(error, BedrockError::ContextWindowExceeded));

    let state = server.state.read().await;
    assert_eq!(state.bedrock_count_token_requests.len(), 1);
    assert!(
        state.bedrock_text_requests.is_empty(),
        "no billed Converse call may happen once the budget is exceeded"
    );
}

#[tokio::test]
async fn truncated_extraction_is_an_error_not_a_partial_sidecar() {
    let server = MockServer::spawn().await;
    server
        .state
        .write()
        .await
        .bedrock_text_responses
        .push(ScriptedBedrockResponse::success(serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{"text": "# Partial extraction"}]}},
            "stopReason": "max_tokens",
            "usage": {"inputTokens": 5000, "outputTokens": 16384}
        })));

    let error = extract_document_text(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        b"%PDF-1.4 fake",
        "scan.pdf",
        DocumentFormat::Pdf,
        DEFAULT_EXTRACTION_PROMPT,
    )
    .await
    .expect_err("truncated extraction must fail");
    assert!(matches!(
        error,
        BedrockError::ResponseTruncated {
            max_output_tokens
        } if max_output_tokens == EXTRACT_MAX_OUTPUT_TOKENS
    ));

    let state = server.state.read().await;
    assert_eq!(
        state.bedrock_text_requests[0]["inferenceConfig"]["maxTokens"],
        EXTRACT_MAX_OUTPUT_TOKENS
    );
}
