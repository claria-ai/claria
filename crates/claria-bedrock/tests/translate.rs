//! Translation flow tests: the envelope arrives as a forced tool call, and
//! the returned index set must exactly match the requested set.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_bedrock::{
    error::BedrockError,
    translate::{
        SUBMIT_TRANSLATIONS_TOOL, TRANSLATE_MAX_OUTPUT_TOKENS, TranslationRequest,
        translate_segments,
    },
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};

const MODEL_ID: &str = "us.anthropic.claude-haiku-test";

fn sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let credentials = Credentials::new("test", "test", None, None, "test");
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

fn requests() -> Vec<TranslationRequest> {
    vec![
        TranslationRequest {
            index: 0,
            language_code: "es-US".to_string(),
            source_text: "Hola doctor".to_string(),
        },
        TranslationRequest {
            index: 2,
            language_code: "es-US".to_string(),
            source_text: "Me duele la cabeza".to_string(),
        },
    ]
}

#[tokio::test]
async fn translation_uses_a_forced_tool_call_without_temperature() {
    let server = MockServer::spawn().await;
    let (outputs, usage) = translate_segments(&sdk_config(&server.endpoint), MODEL_ID, &requests())
        .await
        .expect("translate");

    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0].index, 0);
    assert_eq!(outputs[0].translation, "[EN] Hola doctor");
    assert_eq!(outputs[1].index, 2);
    assert_eq!(outputs[1].translation, "[EN] Me duele la cabeza");
    assert!(usage.expect("usage").input_tokens > 0);

    let state = server.state.read().await;
    assert_eq!(state.bedrock_tool_requests.len(), 1);
    let request = &state.bedrock_tool_requests[0];
    assert_eq!(
        request["toolConfig"]["toolChoice"]["tool"]["name"],
        SUBMIT_TRANSLATIONS_TOOL
    );
    let tools = request["toolConfig"]["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["toolSpec"]["name"], SUBMIT_TRANSLATIONS_TOOL);
    // Never set temperature/top_p; always bound the output.
    assert!(request["inferenceConfig"].get("temperature").is_none());
    assert!(request["inferenceConfig"].get("topP").is_none());
    assert_eq!(
        request["inferenceConfig"]["maxTokens"],
        TRANSLATE_MAX_OUTPUT_TOKENS
    );
}

#[tokio::test]
async fn missing_translation_indexes_are_an_error_not_a_silent_gap() {
    let server = MockServer::spawn().await;
    server
        .state
        .write()
        .await
        .bedrock_tool_responses
        .push(ScriptedBedrockResponse::success(serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [{
                "toolUse": {
                    "toolUseId": "forced-1",
                    "name": "submit_translations",
                    "input": {"translations": [{"index": 0, "translation": "Hello doctor"}]}
                }
            }]}},
            "stopReason": "tool_use",
            "usage": {"inputTokens": 10, "outputTokens": 4}
        })));

    let error = translate_segments(&sdk_config(&server.endpoint), MODEL_ID, &requests())
        .await
        .expect_err("gap must fail");
    assert!(
        matches!(
            &error,
            BedrockError::SchemaViolation(message) if message.contains("missing") && message.contains('2')
        ),
        "{error}"
    );
}

#[tokio::test]
async fn duplicate_and_unrequested_indexes_are_rejected() {
    for translations in [
        serde_json::json!([
            {"index": 0, "translation": "Hello"},
            {"index": 0, "translation": "Hello again"},
            {"index": 2, "translation": "My head hurts"}
        ]),
        serde_json::json!([
            {"index": 0, "translation": "Hello"},
            {"index": 1, "translation": "Not requested"},
            {"index": 2, "translation": "My head hurts"}
        ]),
    ] {
        let server = MockServer::spawn().await;
        server
            .state
            .write()
            .await
            .bedrock_tool_responses
            .push(ScriptedBedrockResponse::success(serde_json::json!({
                "output": {"message": {"role": "assistant", "content": [{
                    "toolUse": {
                        "toolUseId": "forced-1",
                        "name": "submit_translations",
                        "input": {"translations": translations}
                    }
                }]}},
                "stopReason": "tool_use"
            })));

        let error = translate_segments(&sdk_config(&server.endpoint), MODEL_ID, &requests())
            .await
            .expect_err("invalid index set");
        assert!(matches!(error, BedrockError::SchemaViolation(_)), "{error}");
    }
}

#[tokio::test]
async fn empty_requests_short_circuit_without_a_bedrock_call() {
    let server = MockServer::spawn().await;
    let (outputs, usage) = translate_segments(&sdk_config(&server.endpoint), MODEL_ID, &[])
        .await
        .expect("empty");
    assert!(outputs.is_empty());
    assert!(usage.is_none());
    assert!(server.state.read().await.bedrock_tool_requests.is_empty());
}
