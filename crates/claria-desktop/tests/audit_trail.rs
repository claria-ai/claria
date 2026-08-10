//! The desktop audit policy, end to end through the real console layer: an
//! audit event reaches durable storage and the exported log, and a sink that
//! is unreachable degrades to a visible error instead of failing the command.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use tracing_subscriber::prelude::*;

use claria_desktop::console::{ConsoleBuffer, ConsoleLayer};
use claria_mock_aws::testing::MockServer;
use claria_storage::audit::AuditEvent;

const BUCKET: &str = "123456789012-claria-data";

fn build_sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let creds = Credentials::new(
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        None,
        None,
        "claria-test",
    );
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(creds))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

fn chat_turn_event() -> AuditEvent {
    AuditEvent::new(
        "chat_message",
        "client",
        uuid::Uuid::nil().to_string(),
        "123456789012",
    )
    .with_details(serde_json::json!({
        "input_tokens": 1234,
        "output_tokens": 88,
        "cost_usd": 0.0042,
    }))
}

fn today() -> jiff::civil::Date {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
}

#[tokio::test]
async fn a_recorded_event_reaches_s3_and_carries_its_details_into_the_exported_log() {
    let buffer = ConsoleBuffer::new();
    let subscriber =
        tracing_subscriber::registry().with(ConsoleLayer::new(buffer.clone()).with_filter(
            tracing_subscriber::EnvFilter::new("info,claria_storage=trace"),
        ));
    let _guard = tracing::subscriber::set_default(subscriber);

    let mock = MockServer::spawn().await;
    let sdk = build_sdk_config(&mock.endpoint);
    claria_storage::client::from_config(&sdk)
        .create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");

    let event = chat_turn_event();
    let event_id = event.event_id;
    claria_desktop::audit::record(&sdk, BUCKET, event).await;

    let stored = claria_storage::audit::read_day(&sdk, BUCKET, today())
        .await
        .expect("read day");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].event_id, event_id);
    assert_eq!(
        stored[0].details.as_ref().expect("details")["input_tokens"],
        1234
    );

    // The console mirror is a one-line summary: action and UUID reference,
    // never the details payload — that lives only in the durable S3 object.
    let text = buffer.to_text();
    let line = text
        .lines()
        .find(|l| l.contains("audit event"))
        .unwrap_or_else(|| panic!("no audit line in export:\n{text}"));
    assert!(line.contains("audit.action=chat_message"), "{line}");
    assert!(
        line.contains(&format!("audit.resource_id={}", uuid::Uuid::nil())),
        "{line}"
    );
    assert!(!text.contains("input_tokens"), "{text}");
    assert!(!text.contains("cost_usd"), "{text}");
}

#[tokio::test]
async fn an_unwritable_sink_surfaces_an_error_without_failing_the_caller() {
    let buffer = ConsoleBuffer::new();
    let subscriber = tracing_subscriber::registry().with(
        ConsoleLayer::new(buffer.clone()).with_filter(tracing_subscriber::EnvFilter::new("info")),
    );
    let _guard = tracing::subscriber::set_default(subscriber);

    let mock = MockServer::spawn().await;
    let sdk = build_sdk_config(&mock.endpoint);
    // No bucket created: every audit PUT will fail.

    let event = chat_turn_event();
    let event_id = event.event_id;

    // Returns normally — the chat turn that produced this event must not be
    // reported as failed just because its receipt could not be filed.
    claria_desktop::audit::record(&sdk, BUCKET, event).await;

    let text = buffer.to_text();
    assert!(
        text.lines().any(|l| l.contains("ERROR")
            && l.contains("not persisted")
            && l.contains(&event_id.to_string())),
        "the failed audit write was swallowed instead of surfaced:\n{text}"
    );

    // The summary line still records that the event happened, even though it
    // never reached S3.
    assert!(
        text.lines()
            .any(|l| l.contains("audit event") && l.contains("audit.action=chat_message")),
        "the event summary was lost along with the write:\n{text}"
    );
}
