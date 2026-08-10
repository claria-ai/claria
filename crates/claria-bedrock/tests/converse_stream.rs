//! Streamed chat: the accumulator's event-folding rules and the full
//! `ConverseStream` path against the mock's event-stream frames.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_sdk_bedrockruntime::types::{
    ContentBlockDelta, ContentBlockDeltaEvent, ConverseStreamMetadataEvent, ConverseStreamOutput,
    MessageStopEvent, StopReason, TokenUsage,
};
use claria_bedrock::{
    chat::{CacheStrategy, ChatMessage, ChatRole, chat_converse_stream},
    converse::StreamCollector,
    error::BedrockError,
};
use claria_core::model_id::CacheTtlChoice;
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};

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

fn delta_event(text: &str) -> ConverseStreamOutput {
    ConverseStreamOutput::ContentBlockDelta(
        ContentBlockDeltaEvent::builder()
            .delta(ContentBlockDelta::Text(text.to_string()))
            .content_block_index(0)
            .build()
            .expect("delta event"),
    )
}

fn stop_event(stop_reason: StopReason) -> ConverseStreamOutput {
    ConverseStreamOutput::MessageStop(
        MessageStopEvent::builder()
            .stop_reason(stop_reason)
            .build()
            .expect("stop event"),
    )
}

fn metadata_event(input_tokens: i32, output_tokens: i32) -> ConverseStreamOutput {
    ConverseStreamOutput::Metadata(
        ConverseStreamMetadataEvent::builder()
            .usage(
                TokenUsage::builder()
                    .input_tokens(input_tokens)
                    .output_tokens(output_tokens)
                    .total_tokens(input_tokens + output_tokens)
                    .build()
                    .expect("usage"),
            )
            .build(),
    )
}

// ── Accumulator unit tests ──────────────────────────────────────────────────

#[test]
fn collector_accumulates_deltas_and_captures_usage() {
    let mut collector = StreamCollector::new();
    assert_eq!(
        collector.absorb(delta_event("Hello, ")).as_deref(),
        Some("Hello, ")
    );
    assert_eq!(
        collector.absorb(delta_event("world.")).as_deref(),
        Some("world.")
    );
    assert!(collector.absorb(stop_event(StopReason::EndTurn)).is_none());
    assert!(collector.absorb(metadata_event(120, 7)).is_none());

    let outcome = collector.finish(MODEL_ID, 8_192, None).expect("complete stream");
    assert_eq!(outcome.text, "Hello, world.");
    assert_eq!(outcome.stop_reason, "end_turn");
    let usage = outcome.usage.expect("usage captured");
    assert_eq!(usage.input_tokens, 120);
    assert_eq!(usage.output_tokens, 7);
}

#[test]
fn collector_surfaces_truncation_as_a_typed_error() {
    let mut collector = StreamCollector::new();
    collector.absorb(delta_event("partial out"));
    collector.absorb(stop_event(StopReason::MaxTokens));

    let error = collector.finish(MODEL_ID, 8_192, None).expect_err("truncated");
    assert!(matches!(
        error,
        BedrockError::ResponseTruncated {
            max_output_tokens: 8_192
        }
    ));
}

#[test]
fn collector_rejects_a_stream_that_never_stopped() {
    let mut collector = StreamCollector::new();
    collector.absorb(delta_event("cut off mid-"));

    let error = collector.finish(MODEL_ID, 8_192, None).expect_err("no stop");
    assert!(matches!(error, BedrockError::ResponseParse(message)
        if message.contains("messageStop")));
}

#[test]
fn collector_preserves_missing_usage_as_none() {
    let mut collector = StreamCollector::new();
    collector.absorb(delta_event("text"));
    collector.absorb(stop_event(StopReason::EndTurn));

    let outcome = collector.finish(MODEL_ID, 8_192, None).expect("complete");
    assert!(outcome.usage.is_none());
}

// ── End-to-end against the mock's event-stream frames ───────────────────────

#[tokio::test]
async fn streamed_chat_forwards_deltas_and_returns_the_full_text() {
    let server = MockServer::spawn().await;
    let mut deltas: Vec<String> = Vec::new();
    let outcome = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        CacheStrategy::disabled(),
        |delta| deltas.push(delta.to_string()),
    )
    .await
    .expect("streamed chat");

    // The canned response streams as several deltas that concatenate to the
    // returned text.
    assert!(deltas.len() > 1, "expected multiple deltas, got {deltas:?}");
    assert_eq!(deltas.concat(), outcome.text);
    assert!(outcome.text.contains("I understand your question"));
    assert_eq!(outcome.stop_reason, "end_turn");
    assert_eq!(outcome.usage.expect("usage").input_tokens, 150);

    let state = server.state.read().await;
    assert_eq!(state.bedrock_stream_request_count, 1);
    assert_eq!(state.bedrock_text_requests.len(), 1);
    assert_eq!(
        state.bedrock_text_requests[0]["inferenceConfig"]["maxTokens"],
        claria_bedrock::chat::CHAT_MAX_OUTPUT_TOKENS
    );
}

#[tokio::test]
async fn one_hour_chat_caching_marks_system_and_conversation_tail() {
    let server = MockServer::spawn().await;
    // Above the ~4,800-char floor so both cache gates open.
    let system_prompt = "s".repeat(6_000);
    let outcome = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        &system_prompt,
        &user_messages("Hello"),
        CacheStrategy::enabled_for_model(true, CacheTtlChoice::OneHour),
        |_| {},
    )
    .await
    .expect("streamed chat");

    // The recorded usage carries the TTL the request was cached at.
    assert_eq!(
        outcome.usage.expect("usage").cache_ttl,
        Some(CacheTtlChoice::OneHour)
    );

    let state = server.state.read().await;
    let request = &state.bedrock_text_requests[0];
    // System prefix: text block then a cache point carrying the 1h TTL.
    assert_eq!(
        request["system"][1]["cachePoint"],
        serde_json::json!({"type": "default", "ttl": "1h"})
    );
    // Conversation tail: the last message's content ends with the same
    // 1h cache point, so the next turn reads the history from cache.
    let tail = request["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_array()
        .unwrap();
    assert_eq!(
        tail.last().unwrap()["cachePoint"],
        serde_json::json!({"type": "default", "ttl": "1h"}),
        "conversation tail must end with a 1h cache point"
    );
}

#[tokio::test]
async fn five_minute_chat_caching_omits_the_ttl_field() {
    let server = MockServer::spawn().await;
    let system_prompt = "s".repeat(6_000);
    let outcome = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        &system_prompt,
        &user_messages("Hello"),
        CacheStrategy::enabled_for_model(true, CacheTtlChoice::FiveMinutes),
        |_| {},
    )
    .await
    .expect("streamed chat");

    assert_eq!(
        outcome.usage.expect("usage").cache_ttl,
        Some(CacheTtlChoice::FiveMinutes)
    );

    let state = server.state.read().await;
    let request = &state.bedrock_text_requests[0];
    // Default-TTL cache points stay byte-identical to the pre-TTL shape:
    // no `ttl` key at all, on either marker.
    assert_eq!(
        request["system"][1]["cachePoint"],
        serde_json::json!({"type": "default"})
    );
    let tail = request["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_array()
        .unwrap();
    assert_eq!(
        tail.last().unwrap()["cachePoint"],
        serde_json::json!({"type": "default"})
    );
}

#[tokio::test]
async fn uncached_chat_records_no_ttl() {
    let server = MockServer::spawn().await;
    let outcome = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        CacheStrategy::disabled(),
        |_| {},
    )
    .await
    .expect("streamed chat");

    assert_eq!(outcome.usage.expect("usage").cache_ttl, None);
    let state = server.state.read().await;
    assert!(
        !state.bedrock_text_requests[0]
            .to_string()
            .contains("cachePoint")
    );
}

#[tokio::test]
async fn streamed_truncation_is_an_error_not_silent_partial_text() {
    let server = MockServer::spawn().await;
    server
        .state
        .write()
        .await
        .bedrock_text_responses
        .push(ScriptedBedrockResponse {
            status: 200,
            body: serde_json::json!({
                "output": {"message": {"role": "assistant", "content": [{"text": "partial"}]}},
                "stopReason": "max_tokens",
                "usage": {"inputTokens": 10, "outputTokens": 8_192}
            }),
        });

    let error = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        CacheStrategy::disabled(),
        |_| {},
    )
    .await
    .expect_err("truncated stream");
    assert!(matches!(error, BedrockError::ResponseTruncated { .. }));
}

#[tokio::test]
async fn streamed_service_errors_keep_their_classification() {
    let server = MockServer::spawn().await;
    server
        .state
        .write()
        .await
        .bedrock_text_responses
        .push(ScriptedBedrockResponse {
            status: 400,
            body: serde_json::json!({
                "message": "The provided model identifier is invalid.",
                "__type": "ValidationException"
            }),
        });

    let error = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        CacheStrategy::disabled(),
        |_| {},
    )
    .await
    .expect_err("validation error");
    match error {
        BedrockError::Service {
            operation, status, ..
        } => {
            assert_eq!(operation, "chat ConverseStream");
            assert_eq!(status, Some(400));
        }
        other => panic!("expected service error, got {other:?}"),
    }
}
