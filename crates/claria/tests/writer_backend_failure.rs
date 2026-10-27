use std::time::Duration;

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use claria::{ReportMessageRequest, ReportPipelineError, send_report_message};
use claria_bedrock::error::BedrockError;
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

async fn request_path(stream: &mut TcpStream) -> String {
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
    path
}

enum StreamScenario {
    FailedTrailer,
    RecoverableException,
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
            aws_smithy_eventstream::frame::write_message_to(&message, &mut frames)
                .expect("stream exception frame");
            frames
        }
        StreamScenario::RecoverableException => {
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

async fn serve_streams(
    listener: TcpListener,
    mut stop: oneshot::Receiver<()>,
    scenario: StreamScenario,
) -> Vec<String> {
    let mut requests = Vec::new();
    let mut converse_calls = 0;
    loop {
        let (mut stream, _) = tokio::select! {
            result = listener.accept() => result.expect("local request"),
            _ = &mut stop => break,
        };
        let path = request_path(&mut stream).await;
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
            let trailer = match scenario {
                // The valid messageStop precedes a truncated HTTP chunk, so usage never arrives.
                StreamScenario::FailedTrailer => b"\r\n4\r\nx".as_slice(),
                StreamScenario::RecoverableException => b"\r\n0\r\n\r\n".as_slice(),
            };
            stream.write_all(trailer).await.expect("stream trailer");
        }
        stream.shutdown().await.expect("close fixture connection");
        requests.push(path);
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
    assert!(requests[0].ends_with("/count-tokens"));
    assert!(requests[1].ends_with("/converse-stream"));
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
    assert!(requests[0].ends_with("/count-tokens"));
    assert!(requests[1].ends_with("/converse-stream"));
    assert!(requests[2].ends_with("/converse-stream"));
    assert_eq!(outcome.assistant_text, "Synthetic recovered answer.");
    assert_eq!(outcome.attempt.converse_calls, 1);
    assert_eq!(outcome.attempt.tool_uses, 0);
    assert_eq!(outcome.attempt.usage.input_tokens, 13);
    assert_eq!(outcome.attempt.usage.output_tokens, 5);
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
