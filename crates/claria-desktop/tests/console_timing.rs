//! Proves the log-export pipeline carries timing data: an instrumented
//! `claria-storage` call against the in-process mock S3 must land in the
//! console ring buffer as an entry with `elapsed_ms`.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use tracing_subscriber::prelude::*;

use claria_desktop::console::{ConsoleBuffer, ConsoleLayer};
use claria_mock_aws::testing::MockServer;

const BUCKET: &str = "claria-test-bucket";
const KEY: &str = "records/00000000-0000-0000-0000-000000000000/note.txt";

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

#[tokio::test]
async fn instrumented_s3_calls_export_elapsed_ms() {
    // Mirror the app wiring in main.rs: per-layer filters. The export layer
    // admits claria_* trace spans on top of an info baseline; a second layer at
    // the plain "info" level stands in for the fmt/terminal view and must NOT
    // see the trace-level timing spans.
    let export_buffer = ConsoleBuffer::new();
    let default_buffer = ConsoleBuffer::new();
    let subscriber = tracing_subscriber::registry()
        .with(
            ConsoleLayer::new(export_buffer.clone()).with_filter(
                tracing_subscriber::EnvFilter::new(
                    "info,claria_storage=trace,claria_bedrock=trace,claria_desktop=trace",
                ),
            ),
        )
        .with(
            ConsoleLayer::new(default_buffer.clone())
                .with_filter(tracing_subscriber::EnvFilter::new("info")),
        );
    let _guard = tracing::subscriber::set_default(subscriber);

    let mock = MockServer::spawn().await;
    let sdk = build_sdk_config(&mock.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");

    claria_storage::objects::put_object(&s3, BUCKET, KEY, b"hello".to_vec(), Some("text/plain"))
        .await
        .expect("put object");
    let got = claria_storage::objects::get_object(&s3, BUCKET, KEY)
        .await
        .expect("get object");
    assert_eq!(got.body, b"hello");
    let keys = claria_storage::objects::list_objects(&s3, BUCKET, "records/")
        .await
        .expect("list objects");
    assert_eq!(keys, vec![KEY.to_string()]);

    // The exported plain-text log (what a user sends to the developer) must
    // carry per-call spans with elapsed_ms, scrubbed key classes, and byte
    // counts. The client-chosen filename is PHI and must never appear.
    let text = export_buffer.to_text();
    let timing_lines: Vec<&str> = text.lines().filter(|l| l.contains("elapsed_ms=")).collect();

    assert!(
        timing_lines.iter().any(|l| {
            l.contains("put_object")
                && l.contains("key=records/00000000-0000-0000-0000-000000000000/<file>")
        }),
        "missing put_object timing with scrubbed key in export:\n{text}"
    );
    assert!(
        !text.contains("note.txt"),
        "record filename leaked into the exported log:\n{text}"
    );
    assert!(
        timing_lines
            .iter()
            .any(|l| l.contains("get_object") && l.contains("bytes=5")),
        "missing get_object timing with byte count in export:\n{text}"
    );
    assert!(
        timing_lines
            .iter()
            .any(|l| l.contains("list_objects") && l.contains("count=1")),
        "missing list_objects timing with result count in export:\n{text}"
    );

    // The default "info" view (fmt/terminal analog) must stay free of the
    // trace-level timing spans.
    let default_text = default_buffer.to_text();
    assert!(
        !default_text.contains("elapsed_ms="),
        "timing spans leaked into the default info-level log:\n{default_text}"
    );
}
