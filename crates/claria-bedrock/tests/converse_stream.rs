//! Streamed chat: the accumulator's event-folding rules and the full
//! `ConverseStream` path against the mock's event-stream frames.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_sdk_bedrockruntime::types::{
    ContentBlockDelta, ContentBlockDeltaEvent, ConverseStreamMetadataEvent, ConverseStreamOutput,
    MessageStopEvent, StopReason, TokenUsage,
};
use claria_bedrock::{
    chat::{
        CHAT_TRUNCATED_NOTICE, CacheStrategy, ChatMessage, ChatRole, ChatStreamOptions,
        chat_converse_stream,
    },
    converse::{STOPPED_BY_USER, StopSignal, StreamCollector},
    error::BedrockError,
    pacing::StreamPacing,
};
use claria_core::{
    model_id::{CacheTtlChoice, ModelCapabilities},
    models::chat_history::MAX_RETAINED_CHAT_MESSAGES,
};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};

const MODEL_ID: &str = "us.anthropic.claude-sonnet-test";

/// Options that forward every delta, so a test that asserts on individual
/// deltas is reading the stream and not the pacer.
fn token_paced(cache_strategy: CacheStrategy) -> ChatStreamOptions {
    ChatStreamOptions {
        cache_strategy,
        pacing: StreamPacing::Token,
        ..ChatStreamOptions::default()
    }
}

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

    let outcome = collector
        .finish(MODEL_ID, 8_192, None)
        .expect("complete stream");
    assert_eq!(outcome.text, "Hello, world.");
    assert_eq!(outcome.stop_reason, "end_turn");
    let usage = outcome.usage.expect("usage captured");
    assert_eq!(usage.input_tokens, 120);
    assert_eq!(usage.output_tokens, 7);
}

/// Truncation is the one incomplete stop reason the collector keeps: the
/// text has already reached the reader, so discarding it would delete an
/// answer they watched arrive.
#[test]
fn collector_keeps_truncated_text_instead_of_discarding_it() {
    let mut collector = StreamCollector::new();
    collector.absorb(delta_event("partial out"));
    collector.absorb(stop_event(StopReason::MaxTokens));

    let outcome = collector
        .finish(MODEL_ID, 8_192, None)
        .expect("truncation is salvaged");
    assert_eq!(outcome.text, "partial out");
    assert_eq!(outcome.stop_reason, "max_tokens");
}

#[test]
fn collector_rejects_a_stream_that_never_stopped() {
    let mut collector = StreamCollector::new();
    collector.absorb(delta_event("cut off mid-"));

    let error = collector
        .finish(MODEL_ID, 8_192, None)
        .expect_err("no stop");
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
        token_paced(CacheStrategy::disabled()),
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
        token_paced(CacheStrategy::enabled_for_model(
            ModelCapabilities::for_id(MODEL_ID),
            CacheTtlChoice::OneHour,
        )),
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
        token_paced(CacheStrategy::enabled_for_model(
            ModelCapabilities::for_id(MODEL_ID),
            CacheTtlChoice::FiveMinutes,
        )),
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
        token_paced(CacheStrategy::disabled()),
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
async fn streamed_truncation_keeps_the_answer_and_labels_it() {
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

    let mut streamed = String::new();
    let outcome = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        token_paced(CacheStrategy::disabled()),
        |delta| streamed.push_str(delta),
    )
    .await
    .expect("truncation is salvaged, not an error");

    assert!(
        outcome.text.starts_with("partial"),
        "partial answer was discarded: {}",
        outcome.text
    );
    assert!(
        outcome.text.contains(CHAT_TRUNCATED_NOTICE),
        "no truncation notice: {}",
        outcome.text
    );
    assert_eq!(outcome.stop_reason, "max_tokens");
    assert_eq!(
        streamed, outcome.text,
        "the live view and the persisted text disagree"
    );
}

/// Context overflow keeps failing: unlike truncation there is no partial
/// answer worth showing, and the reader has to shorten the conversation.
#[tokio::test]
async fn streamed_context_overflow_is_still_a_typed_error() {
    let server = MockServer::spawn().await;
    server
        .state
        .write()
        .await
        .bedrock_text_responses
        .push(ScriptedBedrockResponse {
            status: 200,
            body: serde_json::json!({
                "output": {"message": {"role": "assistant", "content": [{"text": ""}]}},
                "stopReason": "model_context_window_exceeded",
                "usage": {"inputTokens": 10, "outputTokens": 0}
            }),
        });

    let error = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        token_paced(CacheStrategy::disabled()),
        |_| {},
    )
    .await
    .expect_err("context overflow");
    assert!(matches!(error, BedrockError::ContextWindowExceeded));
}

/// A long conversation is bounded before it reaches the wire. The clinician
/// keeps the whole thread on screen and in S3; Bedrock stops being asked to
/// re-read all of it on every turn.
#[tokio::test]
async fn a_long_conversation_is_bounded_before_it_reaches_bedrock() {
    let server = MockServer::spawn().await;

    let mut messages = Vec::new();
    for index in 0..MAX_RETAINED_CHAT_MESSAGES {
        messages.push(ChatMessage {
            role: if index % 2 == 0 {
                ChatRole::User
            } else {
                ChatRole::Assistant
            },
            content: format!("turn {index}"),
        });
    }
    messages.push(ChatMessage {
        role: ChatRole::User,
        content: "the newest question".to_string(),
    });

    chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &messages,
        token_paced(CacheStrategy::disabled()),
        |_| {},
    )
    .await
    .expect("streamed chat");

    let state = server.state.read().await;
    let sent = state.bedrock_text_requests[0]["messages"]
        .as_array()
        .expect("messages array");
    assert!(
        sent.len() < messages.len(),
        "the full history went on the wire: {} messages",
        sent.len()
    );
    // Converse rejects a conversation that opens on an assistant turn.
    assert_eq!(sent[0]["role"], "user");
    // The newest message always survives the cut.
    assert_eq!(
        sent.last().expect("newest message")["content"][0]["text"],
        "the newest question"
    );
}

/// A stream that opens and then dies has to fail, not hang. Nothing in the
/// AWS SDK bounds this: the generated `ConverseStream` operation registers
/// no stalled-stream protection interceptor, and the read timeout covers
/// only the wait for response headers, which have already arrived by then.
///
/// Runs on a paused clock, so the idle bound elapses in virtual time rather
/// than making the suite wait it out.
#[tokio::test(start_paused = true)]
async fn a_stream_that_goes_silent_fails_instead_of_hanging() {
    let server = MockServer::spawn().await;
    server.state.write().await.bedrock_stream_stalls = 1;

    let mut streamed = String::new();
    let error = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        token_paced(CacheStrategy::disabled()),
        |delta| streamed.push_str(delta),
    )
    .await
    .expect_err("a stalled stream must not hang forever");

    match error {
        BedrockError::StreamInterrupted { operation, message } => {
            assert_eq!(operation, "chat ConverseStream");
            assert!(
                message.contains("chat ConverseStream"),
                "the failure does not name the call: {message}"
            );
            assert!(
                message.contains("stopped sending data"),
                "the failure does not say the connection went silent: {message}"
            );
        }
        other => panic!("expected a stream-interrupted error, got {other:?}"),
    }
    // Whatever did arrive before the stall reached the reader.
    assert!(streamed.starts_with("Beginning of a reply"));
}

/// Paragraph pacing is what a clinician sees by default: the reply lands in
/// readable blocks instead of one twitch per token, and the text is
/// identical either way.
#[tokio::test]
async fn paragraph_pacing_delivers_whole_paragraphs() {
    let reply = "The first paragraph runs on for a while so it spans several \
                 wire deltas.\n\nThe second paragraph is also comfortably longer \
                 than one delta.\n\nDone.";
    let server = MockServer::spawn().await;
    server
        .state
        .write()
        .await
        .bedrock_text_responses
        .push(ScriptedBedrockResponse {
            status: 200,
            body: serde_json::json!({
                "output": {"message": {"role": "assistant", "content": [{"text": reply}]}},
                "stopReason": "end_turn",
                "usage": {"inputTokens": 10, "outputTokens": 40}
            }),
        });

    let mut chunks: Vec<String> = Vec::new();
    let outcome = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        ChatStreamOptions {
            pacing: StreamPacing::Paragraph,
            ..ChatStreamOptions::default()
        },
        |chunk| chunks.push(chunk.to_string()),
    )
    .await
    .expect("streamed chat");

    assert_eq!(
        chunks,
        vec![
            "The first paragraph runs on for a while so it spans several wire deltas.\n\n",
            "The second paragraph is also comfortably longer than one delta.\n\n",
            "Done.",
        ]
    );
    assert_eq!(chunks.concat(), outcome.text);
}

/// Streaming off restores the pre-streaming experience: the reply appears
/// once, whole, from the command's return value.
#[tokio::test]
async fn pacing_off_forwards_nothing_but_still_returns_the_reply() {
    let server = MockServer::spawn().await;
    let mut chunks: Vec<String> = Vec::new();
    let outcome = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        ChatStreamOptions {
            pacing: StreamPacing::Off,
            ..ChatStreamOptions::default()
        },
        |chunk| chunks.push(chunk.to_string()),
    )
    .await
    .expect("streamed chat");

    assert!(chunks.is_empty(), "text leaked to the reader: {chunks:?}");
    assert!(outcome.text.contains("I understand your question"));
}

/// Stopping is a choice, not a failure: the text that arrived is kept, the
/// turn is labelled as the reader's doing, and the call returns instead of
/// sitting out the idle bound.
///
/// Runs against a stream that goes silent after its first delta — the case
/// where Stop has to interrupt a parked read rather than slot between two
/// frames. The first delta hands the reader's cue to a separate task, so the
/// stop lands while the stream loop is waiting, not while it is running.
/// Without that interrupt this test hangs for five minutes and then fails.
#[tokio::test]
async fn a_stopped_stream_keeps_what_arrived_and_returns_at_once() {
    let server = MockServer::spawn().await;
    server.state.write().await.bedrock_stream_stalls = 1;

    let stop = StopSignal::new();
    let stopper = stop.clone();
    let (reader_stopped, cue) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = cue.await;
        stopper.stop();
    });

    let mut reader_stopped = Some(reader_stopped);
    let mut streamed = String::new();
    let outcome = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        ChatStreamOptions {
            pacing: StreamPacing::Token,
            stop,
            ..ChatStreamOptions::default()
        },
        |delta| {
            streamed.push_str(delta);
            if let Some(cue) = reader_stopped.take() {
                let _ = cue.send(());
            }
        },
    )
    .await
    .expect("stopping a stream is not an error");

    assert_eq!(outcome.stop_reason, STOPPED_BY_USER);
    assert!(
        outcome.text.starts_with("Beginning of a reply"),
        "the partial answer was discarded: {}",
        outcome.text
    );
    assert_eq!(streamed, outcome.text);
    // The trailing metadata frame never arrived, so the turn is unmetered.
    assert!(outcome.usage.is_none());
}

/// Stopping part-way through a paced reply keeps the paragraphs already read
/// and releases whatever the pacer was still holding, so the bubble on
/// screen and the text that gets persisted are the same characters.
#[tokio::test]
async fn stopping_a_paced_reply_flushes_what_was_held() {
    let reply = "First paragraph, long enough to span several wire deltas.\n\n\
                 Second paragraph, which the reader never asked to see.\n\n\
                 Third paragraph, likewise.";
    let server = MockServer::spawn().await;
    server
        .state
        .write()
        .await
        .bedrock_text_responses
        .push(ScriptedBedrockResponse {
            status: 200,
            body: serde_json::json!({
                "output": {"message": {"role": "assistant", "content": [{"text": reply}]}},
                "stopReason": "end_turn",
                "usage": {"inputTokens": 10, "outputTokens": 40}
            }),
        });

    let stop = StopSignal::new();
    let stopper = stop.clone();
    let mut streamed = String::new();
    let outcome = chat_converse_stream(
        &sdk_config(&server.endpoint),
        MODEL_ID,
        "System prompt",
        &user_messages("Hello"),
        ChatStreamOptions {
            pacing: StreamPacing::Paragraph,
            stop,
            ..ChatStreamOptions::default()
        },
        // The reader stops as soon as the first paragraph appears.
        |chunk| {
            streamed.push_str(chunk);
            stopper.stop();
        },
    )
    .await
    .expect("stopping a stream is not an error");

    assert_eq!(outcome.stop_reason, STOPPED_BY_USER);
    assert!(
        streamed.starts_with("First paragraph"),
        "the first paragraph never reached the reader: {streamed}"
    );
    assert!(
        streamed.len() < reply.len(),
        "the whole reply arrived anyway, so nothing was stopped"
    );
    assert_eq!(streamed, outcome.text);
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
        token_paced(CacheStrategy::disabled()),
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
