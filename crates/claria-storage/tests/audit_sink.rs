//! The durable sink: events must survive the process, land in a time-sliceable
//! key, and report a failed write rather than pretending it succeeded.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_core::s3_keys;
use claria_mock_aws::testing::MockServer;
use claria_storage::{
    audit::{self as sink, AuditEvent, actions},
    error::StorageError,
};

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

async fn create_bucket(sdk: &aws_config::SdkConfig) {
    claria_storage::client::from_config(sdk)
        .create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");
}

fn today() -> jiff::civil::Date {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
}

#[tokio::test]
async fn an_event_round_trips_through_s3_with_its_details_intact() {
    let mock = MockServer::spawn().await;
    let sdk = build_sdk_config(&mock.endpoint);
    create_bucket(&sdk).await;

    let event = AuditEvent::new(actions::CHAT_TURN, "client", "abc", "123456789012").with_details(
        serde_json::json!({
            "chat_id": "11111111-1111-1111-1111-111111111111",
            "input_tokens": 900,
            "output_tokens": 120,
            "cost_usd": 0.0135,
        }),
    );

    let key = sink::write_event(&sdk, BUCKET, &event)
        .await
        .expect("write event");
    assert_eq!(key, s3_keys::audit_event(event.timestamp, event.event_id));

    let read_back = sink::read_day(&sdk, BUCKET, today())
        .await
        .expect("read day");

    assert_eq!(read_back.len(), 1);
    let got = &read_back[0];
    assert_eq!(got.event_id, event.event_id);
    assert_eq!(got.timestamp, event.timestamp);
    assert_eq!(got.action, "chat.turn");
    assert_eq!(got.user_sub, "123456789012");
    // The payload the old `emit()` dropped is now in durable storage.
    assert_eq!(got.details, event.details);
}

#[tokio::test]
async fn each_event_gets_its_own_object_under_the_day_prefix() {
    let mock = MockServer::spawn().await;
    let sdk = build_sdk_config(&mock.endpoint);
    create_bucket(&sdk).await;

    for n in 0..5 {
        let event = AuditEvent::new(actions::CHAT_TURN, "client", "abc", "acct")
            .with_details(serde_json::json!({ "turn": n }));
        sink::write_event(&sdk, BUCKET, &event)
            .await
            .expect("write event");
    }

    let s3 = claria_storage::client::from_config(&sdk);
    let day_prefix = s3_keys::audit_day_prefix(today());
    let keys = claria_storage::objects::list_objects(&s3, BUCKET, &day_prefix)
        .await
        .expect("list day");
    assert_eq!(keys.len(), 5, "one immutable object per event: {keys:?}");

    // Listing the day prefix must not require walking the whole trail, and
    // S3's listing order must already be chronological.
    let events = sink::read_day(&sdk, BUCKET, today())
        .await
        .expect("read day");
    let turns: Vec<i64> = events
        .iter()
        .filter_map(|e| e.details.as_ref()?["turn"].as_i64())
        .collect();
    assert_eq!(turns, vec![0, 1, 2, 3, 4]);

    assert!(
        events.windows(2).all(|w| w[0].timestamp <= w[1].timestamp),
        "listing order is not chronological"
    );
}

#[tokio::test]
async fn a_day_with_no_events_reads_back_empty() {
    let mock = MockServer::spawn().await;
    let sdk = build_sdk_config(&mock.endpoint);
    create_bucket(&sdk).await;

    let yesterday = today().yesterday().expect("yesterday");
    let events = sink::read_day(&sdk, BUCKET, yesterday)
        .await
        .expect("read day");
    assert!(events.is_empty());
}

#[tokio::test]
async fn a_failed_write_is_reported_not_swallowed() {
    let mock = MockServer::spawn().await;
    let sdk = build_sdk_config(&mock.endpoint);
    // Deliberately no bucket: the audit trail has nowhere to land.

    let event = AuditEvent::new(actions::CHAT_TURN, "client", "abc", "acct");
    let err = sink::write_event(&sdk, BUCKET, &event)
        .await
        .expect_err("write must fail against a missing bucket");

    assert!(
        matches!(err, StorageError::PutObject(_)),
        "expected a storage error, got {err:?}"
    );
    // The message has to name the problem — it is what reaches the console.
    assert!(
        err.to_string().contains("S3 PutObject error"),
        "unhelpful error: {err}"
    );
}
