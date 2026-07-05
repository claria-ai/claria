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
    // Mirror the app wiring in main.rs: "info" filter + ConsoleLayer.
    let buffer = ConsoleBuffer::new();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("info"))
        .with(ConsoleLayer::new(buffer.clone()));
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
    // carry per-call spans with elapsed_ms, keys, and byte counts.
    let text = buffer.to_text();
    let timing_lines: Vec<&str> = text.lines().filter(|l| l.contains("elapsed_ms=")).collect();

    assert!(
        timing_lines
            .iter()
            .any(|l| l.contains("put_object") && l.contains(&format!("key={KEY}"))),
        "missing put_object timing in export:\n{text}"
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
}
