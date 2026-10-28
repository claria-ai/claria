use std::{
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
    time::Duration,
};

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use claria::{ReportMessageRequest, ReportPipelineError, ReportTurnProgress, send_report_message};
use claria_bedrock::{converse::StopSignal, error::BedrockError};
use claria_mock_aws::testing::MockServer;
use claria_report_store::{ReportAttemptMetadata, ReportAttemptStatus, ReportFailureCode};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};
use uuid::Uuid;

const BUCKET: &str = "claria-writer-backend-failure-test";
const MODEL_ID: &str = "us.anthropic.claude-sonnet-test";

fn sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(Credentials::new(
            "test", "test", None, None, "test",
        )))
        .endpoint_url(endpoint)
        .retry_config(
            aws_config::retry::RetryConfig::standard()
                .with_max_attempts(2)
                .with_initial_backoff(Duration::from_millis(1))
                .with_max_backoff(Duration::from_millis(1)),
        )
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

fn event(frames: &mut Vec<u8>, event_type: &str, payload: Value) {
    let message = Message::new(payload.to_string().into_bytes())
        .add_header(Header::new(
            ":message-type",
            HeaderValue::String("event".into()),
        ))
        .add_header(Header::new(
            ":event-type",
            HeaderValue::String(event_type.to_string().into()),
        ))
        .add_header(Header::new(
            ":content-type",
            HeaderValue::String("application/json".into()),
        ));
    aws_smithy_eventstream::frame::write_message_to(&message, frames).expect("event frame");
}

struct RecordedRequest {
    path: String,
    body: Value,
}

async fn request_path(stream: &mut TcpStream) -> RecordedRequest {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.expect("request bytes");
        assert!(read > 0, "request ended before its headers");
        request.extend_from_slice(&chunk[..read]);
        assert!(
            request.len() <= 128 * 1024,
            "synthetic request exceeded its bound"
        );
        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = std::str::from_utf8(&request[..header_end]).expect("request headers");
    let path = headers
        .lines()
        .next()
        .expect("request line")
        .split_whitespace()
        .nth(1)
        .expect("request path")
        .to_string();
    let content_length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .expect("SDK JSON request has content length")
        .1
        .trim()
        .parse::<usize>()
        .expect("request content length");
    assert!(
        content_length <= 128 * 1024,
        "synthetic body exceeded its bound"
    );
    while request.len() < header_end + content_length {
        let read = stream.read(&mut chunk).await.expect("request body");
        assert!(read > 0, "request ended before its body");
        request.extend_from_slice(&chunk[..read]);
    }
    RecordedRequest {
        path,
        body: serde_json::from_slice(&request[header_end..header_end + content_length])
            .expect("request JSON"),
    }
}

enum StreamScenario {
    FailedTrailer,
    RecoverableException,
    RecoverableFirstEvent,
    RecoverableProposal,
    SdkRetry,
    CompleteText,
    StopPartialProposal(StopSignal),
}

fn text_frames(text: &str) -> Vec<u8> {
    let mut frames = Vec::new();
    event(&mut frames, "messageStart", json!({"role": "assistant"}));
    event(
        &mut frames,
        "contentBlockDelta",
        json!({"contentBlockIndex": 0, "delta": {"text": text}}),
    );
    event(
        &mut frames,
        "contentBlockStop",
        json!({"contentBlockIndex": 0}),
    );
    frames
}

fn response_frames(scenario: &StreamScenario, call: usize) -> Vec<u8> {
    match scenario {
        StreamScenario::FailedTrailer => {
            let mut frames = text_frames("Synthetic completed answer.");
            event(
                &mut frames,
                "messageStop",
                json!({"stopReason": "end_turn"}),
            );
            frames
        }
        StreamScenario::RecoverableException if call == 1 => {
            let mut frames = text_frames("Synthetic partial answer must disappear.");
            append_retryable_failure(&mut frames);
            frames
        }
        StreamScenario::RecoverableFirstEvent if call == 1 => {
            let mut frames = Vec::new();
            append_retryable_failure(&mut frames);
            frames
        }
        StreamScenario::RecoverableProposal if call <= 2 => {
            let mut frames = proposal_frames();
            if call == 1 {
                append_retryable_failure(&mut frames);
            } else {
                event(
                    &mut frames,
                    "messageStop",
                    json!({"stopReason": "tool_use"}),
                );
                event(
                    &mut frames,
                    "metadata",
                    json!({"usage": {"inputTokens": 17, "outputTokens": 11, "totalTokens": 28}, "metrics": {"latencyMs": 1}}),
                );
            }
            frames
        }
        StreamScenario::StopPartialProposal(_) => proposal_frames(),
        StreamScenario::RecoverableException
        | StreamScenario::RecoverableFirstEvent
        | StreamScenario::RecoverableProposal
        | StreamScenario::SdkRetry
        | StreamScenario::CompleteText => {
            let mut frames = text_frames("Synthetic recovered answer.");
            event(
                &mut frames,
                "messageStop",
                json!({"stopReason": "end_turn"}),
            );
            event(
                &mut frames,
                "metadata",
                json!({
                    "usage": {"inputTokens": 13, "outputTokens": 5, "totalTokens": 18},
                    "metrics": {"latencyMs": 1}
                }),
            );
            frames
        }
    }
}

fn proposal_frames() -> Vec<u8> {
    let mut frames = Vec::new();
    event(&mut frames, "messageStart", json!({"role": "assistant"}));
    event(
        &mut frames,
        "contentBlockStart",
        json!({"contentBlockIndex": 0, "start": {"toolUse": {"toolUseId": "synthetic-proposal", "name": "propose_report_changes"}}}),
    );
    let input = json!({"summary": "Synthetic evidence revision", "operations": [{"kind": "add_section", "position": 0, "heading": "Synthetic findings", "blocks": [{"kind": "paragraph", "text": "Exact synthetic result: 2 → 3."}]}]}).to_string();
    let split = input.floor_char_boundary(input.len() / 2);
    for fragment in [&input[..split], &input[split..]] {
        event(
            &mut frames,
            "contentBlockDelta",
            json!({"contentBlockIndex": 0, "delta": {"toolUse": {"input": fragment}}}),
        );
    }
    event(
        &mut frames,
        "contentBlockStop",
        json!({"contentBlockIndex": 0}),
    );
    frames
}

fn append_retryable_failure(frames: &mut Vec<u8>) {
    let message = Message::new(
        json!({"message": "Synthetic retryable stream failure", "originalStatusCode": 500})
            .to_string()
            .into_bytes(),
    )
    .add_header(Header::new(
        ":message-type",
        HeaderValue::String("exception".into()),
    ))
    .add_header(Header::new(
        ":exception-type",
        HeaderValue::String("modelStreamErrorException".into()),
    ))
    .add_header(Header::new(
        ":content-type",
        HeaderValue::String("application/json".into()),
    ));
    aws_smithy_eventstream::frame::write_message_to(&message, frames)
        .expect("stream exception frame");
}

async fn serve_streams(
    listener: TcpListener,
    mut stop: oneshot::Receiver<()>,
    scenario: StreamScenario,
) -> Vec<RecordedRequest> {
    let mut requests = Vec::new();
    let mut converse_calls = 0;
    loop {
        let (mut stream, _) = tokio::select! {
            result = listener.accept() => result.expect("local request"),
            _ = &mut stop => break,
        };
        let request = request_path(&mut stream).await;
        let path = &request.path;
        if path.ends_with("/count-tokens") {
            let body = br#"{"inputTokens": 100}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .expect("CountTokens headers");
            stream.write_all(body).await.expect("CountTokens body");
        } else {
            assert!(
                path.ends_with("/converse-stream"),
                "unexpected local path: {path}"
            );
            converse_calls += 1;
            if matches!(scenario, StreamScenario::SdkRetry) && converse_calls == 1 {
                let body =
                    br#"{"__type":"ServiceUnavailableException","message":"Synthetic SDK retry"}"#;
                stream.write_all(format!("HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nx-amzn-errortype: ServiceUnavailableException\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.expect("retryable response headers");
                stream
                    .write_all(body)
                    .await
                    .expect("retryable response body");
                stream.shutdown().await.expect("close retryable response");
                requests.push(request);
                continue;
            }
            let frames = response_frames(&scenario, converse_calls);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/vnd.amazon.eventstream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("stream headers");
            stream
                .write_all(format!("{:x}\r\n", frames.len()).as_bytes())
                .await
                .expect("stream chunk length");
            stream
                .write_all(&frames)
                .await
                .expect("completed model events");
            if let StreamScenario::StopPartialProposal(signal) = &scenario {
                stream
                    .write_all(b"\r\n")
                    .await
                    .expect("partial response chunk complete");
                signal.stop();
                (&mut stop)
                    .await
                    .expect("stop fixture after interrupted turn");
                requests.push(request);
                break;
            }
            let trailer = match scenario {
                // The valid messageStop precedes a truncated HTTP chunk, so usage never arrives.
                StreamScenario::FailedTrailer => b"\r\n4\r\nx".as_slice(),
                StreamScenario::RecoverableException
                | StreamScenario::RecoverableFirstEvent
                | StreamScenario::RecoverableProposal
                | StreamScenario::SdkRetry
                | StreamScenario::CompleteText => b"\r\n0\r\n\r\n".as_slice(),
                StreamScenario::StopPartialProposal(_) => {
                    unreachable!("interrupted fixture has no terminal chunk")
                }
            };
            stream.write_all(trailer).await.expect("stream trailer");
        }
        stream.shutdown().await.expect("close fixture connection");
        requests.push(request);
    }
    requests
}

async fn setup() -> (MockServer, aws_sdk_s3::Client, Uuid) {
    let storage = MockServer::spawn().await;
    let s3 = claria_storage::client::from_config(&sdk_config(&storage.endpoint));
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create local bucket");
    let client_id = Uuid::new_v4();
    let now = jiff::Timestamp::now();
    let client = claria_core::models::client::Client {
        id: client_id,
        name: "Synthetic writer fixture".to_string(),
        created_at: now,
        updated_at: now,
    };
    claria_storage::objects::put_object(
        &s3,
        BUCKET,
        &claria_core::s3_keys::client(client_id),
        serde_json::to_vec(&client).expect("synthetic client"),
        Some("application/json"),
    )
    .await
    .expect("store synthetic client");
    (storage, s3, client_id)
}

#[tokio::test]
async fn a_stream_failure_after_message_stop_is_not_retried_or_committed() {
    let (_storage, s3, client_id) = setup().await;
    let before = claria_report_store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("initial workspace");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("local Bedrock listener");
    let sdk = sdk_config(&format!(
        "http://{}",
        listener.local_addr().expect("listener address")
    ));
    let (stop, stopped) = oneshot::channel();
    let exercise = async {
        let result = send_report_message(
            &sdk,
            &s3,
            BUCKET,
            client_id,
            0,
            MODEL_ID,
            ReportMessageRequest::new("Answer using this synthetic fixture."),
        )
        .await;
        stop.send(()).expect("stop local Bedrock listener");
        result
    };
    let (requests, result) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            serve_streams(listener, stopped, StreamScenario::FailedTrailer),
            exercise
        )
    })
    .await
    .expect("bounded local writer call");

    assert_eq!(
        requests.len(),
        2,
        "a completed model response must not be replayed"
    );
    assert!(requests[0].path.ends_with("/count-tokens"));
    assert!(requests[1].path.ends_with("/converse-stream"));
    let error = result.expect_err("the missing trailer cannot produce a completed turn");
    let ReportPipelineError::TurnFailed {
        code,
        attempt,
        source,
        ..
    } = error
    else {
        panic!("expected a failed writer attempt");
    };
    assert_eq!(code, ReportFailureCode::Bedrock);
    assert!(matches!(
        source
            .as_deref()
            .and_then(|error| error.downcast_ref::<BedrockError>()),
        Some(BedrockError::CompletedStreamFailed(_))
    ));
    assert_eq!(attempt.status, ReportAttemptStatus::Aborted);
    assert!(!attempt.usage_complete);
    assert_eq!(attempt.converse_calls, 0);
    assert_eq!(attempt.tool_uses, 0);
    let attempt_key = claria_core::s3_keys::report_attempt(client_id, attempt.attempt_id);
    let (stored, _): (ReportAttemptMetadata, String) =
        claria_storage::state::load_state(&s3, BUCKET, &attempt_key)
            .await
            .expect("durable failed-attempt receipt");
    assert_eq!(stored.status, ReportAttemptStatus::Aborted);
    assert!(!stored.usage_complete);
    assert_eq!(stored.failure_code, Some(ReportFailureCode::Bedrock));

    let after = claria_report_store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("unchanged workspace");
    assert_eq!(after.draft, before.draft);
    assert_eq!(after.session, before.session);
    assert!(after.session.pending_proposal.is_none());
}

#[tokio::test]
async fn a_retryable_stream_exception_discards_partial_text_and_retries_once() {
    let (_storage, s3, client_id) = setup().await;
    let before = claria_report_store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("initial workspace");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("local Bedrock listener");
    let sdk = sdk_config(&format!(
        "http://{}",
        listener.local_addr().expect("listener address")
    ));
    let (stop, stopped) = oneshot::channel();
    let exercise = async {
        let result = send_report_message(
            &sdk,
            &s3,
            BUCKET,
            client_id,
            0,
            MODEL_ID,
            ReportMessageRequest::new("Answer using this synthetic fixture."),
        )
        .await;
        stop.send(()).expect("stop local Bedrock listener");
        result
    };
    let (requests, result) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            serve_streams(listener, stopped, StreamScenario::RecoverableException),
            exercise
        )
    })
    .await
    .expect("bounded local writer retry");

    let outcome = result.expect("the incomplete stream is retryable");
    assert_eq!(requests.len(), 3);
    assert!(requests[0].path.ends_with("/count-tokens"));
    assert!(requests[1].path.ends_with("/converse-stream"));
    assert!(requests[2].path.ends_with("/converse-stream"));
    assert_eq!(outcome.assistant_text, "Synthetic recovered answer.");
    assert_eq!(outcome.attempt.converse_calls, 1);
    assert_eq!(outcome.attempt.tool_uses, 0);
    assert_eq!(outcome.attempt.usage.input_tokens, 13);
    assert_eq!(outcome.attempt.usage.output_tokens, 5);
    assert!(
        !outcome.attempt.usage_complete,
        "a recovered failed physical call leaves its usage unknown"
    );
    assert_eq!(
        requests[1].body, requests[2].body,
        "a retry replays the identical completed conversation"
    );
    let attempt_key = claria_core::s3_keys::report_attempt(client_id, outcome.attempt.attempt_id);
    let (stored, _): (ReportAttemptMetadata, String) =
        claria_storage::state::load_state(&s3, BUCKET, &attempt_key)
            .await
            .expect("durable recovered-attempt receipt");
    assert!(!stored.usage_complete);
    assert_eq!(stored.usage, outcome.attempt.usage);
    assert_eq!(outcome.workspace.draft, before.draft);
    assert!(outcome.workspace.session.pending_proposal.is_none());
    assert_eq!(outcome.workspace.session.turns.len(), 1);
    let after = claria_report_store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("completed workspace");
    assert_eq!(after.session, outcome.workspace.session);
    let history = serde_json::to_string(&after.session).expect("saved conversation");
    assert!(!history.contains("Synthetic partial answer"));
    assert!(!history.contains("Synthetic retryable stream failure"));
    assert!(history.contains("Synthetic recovered answer."));
}

async fn exercise_streams(
    s3: &aws_sdk_s3::Client,
    client_id: Uuid,
    scenario: StreamScenario,
    request: ReportMessageRequest<'_>,
) -> (
    Vec<RecordedRequest>,
    Result<claria::ReportTurnOutcome, ReportPipelineError>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("local Bedrock listener");
    let sdk = sdk_config(&format!(
        "http://{}",
        listener.local_addr().expect("listener address")
    ));
    let (shutdown, stopped) = oneshot::channel();
    let exercise = async {
        let result = send_report_message(&sdk, s3, BUCKET, client_id, 0, MODEL_ID, request).await;
        shutdown.send(()).expect("stop local Bedrock listener");
        result
    };
    tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(serve_streams(listener, stopped, scenario), exercise)
    })
    .await
    .expect("bounded local writer scenario")
}

#[tokio::test]
async fn a_transient_exception_as_the_first_event_is_retried_without_losing_its_classification() {
    let (_storage, s3, client_id) = setup().await;
    let (requests, result) = exercise_streams(
        &s3,
        client_id,
        StreamScenario::RecoverableFirstEvent,
        ReportMessageRequest::new("Answer using this synthetic fixture."),
    )
    .await;
    let outcome = result.expect("the first event's transient exception remains retryable");
    assert_eq!(requests.len(), 3);
    assert!(requests[0].path.ends_with("/count-tokens"));
    assert!(requests[1].path.ends_with("/converse-stream"));
    assert!(requests[2].path.ends_with("/converse-stream"));
    assert_eq!(requests[1].body, requests[2].body);
    assert_eq!(outcome.assistant_text, "Synthetic recovered answer.");
    assert_eq!(outcome.attempt.converse_calls, 1);
    assert!(!outcome.attempt.usage_complete);
    let after = claria_report_store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("completed workspace");
    assert_eq!(after.session, outcome.workspace.session);
}

#[tokio::test]
async fn stopping_an_unfinished_tool_stream_keeps_usage_unknown_and_applies_nothing() {
    let (_storage, s3, client_id) = setup().await;
    let before = claria_report_store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("initial workspace");
    let stop = StopSignal::new();
    let (requests, result) = exercise_streams(
        &s3,
        client_id,
        StreamScenario::StopPartialProposal(stop.clone()),
        ReportMessageRequest::new("Draft a synthetic findings section.").with_stop(&stop),
    )
    .await;
    assert_eq!(requests.len(), 2);
    let error = result.expect_err("unfinished proposal stopped");
    assert!(error.is_stopped());
    let attempt = error.attempt().expect("stopped attempt");
    assert_eq!(attempt.status, ReportAttemptStatus::Stopped);
    assert!(!attempt.usage_complete);
    assert_eq!((attempt.converse_calls, attempt.tool_uses), (0, 0));
    let key = claria_core::s3_keys::report_attempt(client_id, attempt.attempt_id);
    let (stored, _): (ReportAttemptMetadata, String) =
        claria_storage::state::load_state(&s3, BUCKET, &key)
            .await
            .expect("durable stopped attempt");
    assert!(!stored.usage_complete);
    assert_eq!(stored.status, ReportAttemptStatus::Stopped);
    let after = claria_report_store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("unchanged workspace");
    assert_eq!(after.draft, before.draft);
    assert_eq!(after.session, before.session);
    assert!(after.session.pending_proposal.is_none());
}

#[tokio::test]
async fn stopping_when_retry_backoff_is_announced_prevents_another_physical_call() {
    let (_storage, s3, client_id) = setup().await;
    let stop = StopSignal::new();
    let retry_seen = AtomicBool::new(false);
    let progress = |event: ReportTurnProgress| {
        if matches!(event, ReportTurnProgress::ModelCallRetrying { .. }) {
            retry_seen.store(true, Ordering::SeqCst);
            stop.stop();
        }
    };
    let (requests, result) = exercise_streams(
        &s3,
        client_id,
        StreamScenario::RecoverableException,
        ReportMessageRequest::new("Answer using a synthetic fixture.")
            .with_stop(&stop)
            .with_progress(&progress),
    )
    .await;
    assert!(retry_seen.load(Ordering::SeqCst));
    assert_eq!(
        requests.len(),
        2,
        "one CountTokens call and one failed physical Converse call"
    );
    assert!(requests[1].path.ends_with("/converse-stream"));
    let error = result.expect_err("stop cancels retry delay");
    assert!(error.is_stopped());
    let attempt = error.attempt().expect("stopped attempt");
    assert!(!attempt.usage_complete);
    assert_eq!((attempt.converse_calls, attempt.tool_uses), (0, 0));
    let after = claria_report_store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("unchanged workspace");
    assert!(after.session.turns.is_empty());
    assert!(after.session.pending_proposal.is_none());
}

#[tokio::test]
async fn retrying_an_unfinished_proposal_replays_identical_input_and_stages_one_exact_effect() {
    let (_storage, s3, client_id) = setup().await;
    let (requests, result) = exercise_streams(
        &s3,
        client_id,
        StreamScenario::RecoverableProposal,
        ReportMessageRequest::new("Draft a synthetic findings section."),
    )
    .await;
    let outcome = result.expect("proposal retry recovers");
    assert_eq!(
        requests.len(),
        4,
        "one CountTokens call, failed proposal, complete proposal and closing answer"
    );
    assert_eq!(requests[1].body, requests[2].body);
    assert_eq!(
        (outcome.attempt.converse_calls, outcome.attempt.tool_uses),
        (2, 1)
    );
    assert!(!outcome.attempt.usage_complete);
    assert_eq!(outcome.attempt.usage.input_tokens, 30);
    assert_eq!(outcome.attempt.usage.output_tokens, 16);
    assert_eq!(outcome.workspace.draft.revision, 0);
    let proposal = outcome
        .workspace
        .session
        .pending_proposal
        .as_ref()
        .expect("single staged proposal");
    assert_eq!(proposal.operations.len(), 1);
    assert_eq!(proposal.tool_use_id, "synthetic-proposal");
    assert_eq!(proposal.proposed_content.sections.len(), 1);
    assert_eq!(
        proposal.proposed_content.sections[0].blocks,
        vec![claria_core::models::report::ReportBlock::Paragraph {
            text: "Exact synthetic result: 2 → 3.".to_owned()
        }]
    );
    let closing_messages = requests[3].body["messages"]
        .as_array()
        .expect("closing conversation");
    let result_blocks = closing_messages.last().expect("tool result message")["content"]
        .as_array()
        .expect("results");
    let results: Vec<_> = result_blocks
        .iter()
        .filter_map(|block| block.get("toolResult"))
        .collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["toolUseId"], "synthetic-proposal");
    assert_eq!(results[0]["status"], "success");
    let after = claria_report_store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("durable proposal");
    assert_eq!(after.session, outcome.workspace.session);
}

#[tokio::test]
async fn an_sdk_internal_retry_preserves_known_tokens_but_marks_call_and_attempt_usage_incomplete()
{
    let (_storage, s3, client_id) = setup().await;
    let logical_calls = AtomicU32::new(0);
    let progress = |event: ReportTurnProgress| {
        if matches!(event, ReportTurnProgress::ModelCallStarted { .. }) {
            logical_calls.fetch_add(1, Ordering::SeqCst);
        }
    };
    let (requests, result) = exercise_streams(
        &s3,
        client_id,
        StreamScenario::SdkRetry,
        ReportMessageRequest::new("Answer from this synthetic fixture.").with_progress(&progress),
    )
    .await;
    let outcome = result.expect("SDK recovered inside one host call");
    assert_eq!(logical_calls.load(Ordering::SeqCst), 1);
    let physical: Vec<_> = requests
        .iter()
        .filter(|request| request.path.ends_with("/converse-stream"))
        .collect();
    assert_eq!(physical.len(), 2);
    assert_eq!(physical[0].body, physical[1].body);
    assert_eq!(outcome.attempt.converse_calls, 1);
    assert_eq!(outcome.assistant_text, "Synthetic recovered answer.");
    assert_eq!(outcome.attempt.usage.input_tokens, 13);
    assert_eq!(outcome.attempt.usage.output_tokens, 5);
    assert!(!outcome.attempt.usage_complete);
    let call_key =
        claria_core::s3_keys::report_call_usage(client_id, outcome.attempt.attempt_id, 1);
    let (receipt, _): (claria_report_store::ReportCallUsageRecord, String) =
        claria_storage::state::load_state(&s3, BUCKET, &call_key)
            .await
            .expect("immutable completed-call receipt");
    assert!(!receipt.usage_complete);
    let known = receipt.usage.as_ref().expect("known final usage");
    assert_eq!((known.input_tokens, known.output_tokens), (13, 5));
    assert_eq!(known.cost_usd, outcome.attempt.usage.cost_usd);
    let attempt_key = claria_core::s3_keys::report_attempt(client_id, outcome.attempt.attempt_id);
    let (attempt, _): (ReportAttemptMetadata, String) =
        claria_storage::state::load_state(&s3, BUCKET, &attempt_key)
            .await
            .expect("durable attempt receipt");
    assert!(!attempt.usage_complete);
    assert_eq!(attempt.usage, outcome.attempt.usage);
}

#[tokio::test]
async fn one_sdk_attempt_retains_complete_usage_and_its_known_tokens() {
    let (_storage, s3, client_id) = setup().await;
    let (requests, result) = exercise_streams(
        &s3,
        client_id,
        StreamScenario::CompleteText,
        ReportMessageRequest::new("Answer from this synthetic fixture."),
    )
    .await;
    let outcome = result.expect("single model response");
    assert_eq!(
        requests.len(),
        2,
        "one CountTokens and one Converse request"
    );
    assert!(outcome.attempt.usage_complete);
    assert_eq!(
        (
            outcome.attempt.usage.input_tokens,
            outcome.attempt.usage.output_tokens
        ),
        (13, 5)
    );
    let key = claria_core::s3_keys::report_call_usage(client_id, outcome.attempt.attempt_id, 1);
    let (receipt, _): (claria_report_store::ReportCallUsageRecord, String) =
        claria_storage::state::load_state(&s3, BUCKET, &key)
            .await
            .expect("completed-call receipt");
    assert!(receipt.usage_complete);
}
