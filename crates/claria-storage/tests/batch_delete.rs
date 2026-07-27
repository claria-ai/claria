//! Batched deletion against the in-process mock S3.
//!
//! Deletes go out through `DeleteObjects`, which takes up to 1,000 objects per
//! request and reports per-object failures inside an HTTP 200 — a shape that
//! reads as success unless the response body is inspected.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_sdk_s3::primitives::ByteStream;
use claria_mock_aws::{state::ObjectVersion, testing::MockServer};
use claria_storage::{error::StorageError, objects};

const BUCKET: &str = "claria-test-bucket";

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

async fn put(s3: &aws_sdk_s3::Client, key: &str) {
    s3.put_object()
        .bucket(BUCKET)
        .key(key)
        .body(ByteStream::from_static(b"data"))
        .send()
        .await
        .expect("put object");
}

/// Seed the mock's object map directly. Thousands of round trips through the
/// SDK would prove nothing extra about the chunking under test.
async fn seed_versions(server: &MockServer, keys: impl IntoIterator<Item = String>) {
    let mut st = server.state.write().await;
    for key in keys {
        st.objects
            .entry((BUCKET.to_string(), key))
            .or_default()
            .push(ObjectVersion {
                version_id: uuid::Uuid::new_v4().to_string(),
                body: bytes::Bytes::from_static(b"data"),
                content_type: "text/plain".to_string(),
                etag: "etag".to_string(),
                last_modified: "2026-01-01T00:00:00Z".to_string(),
                is_delete_marker: false,
            });
    }
}

async fn create_versioned_bucket(s3: &aws_sdk_s3::Client) {
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");
    s3.put_bucket_versioning()
        .bucket(BUCKET)
        .versioning_configuration(
            aws_sdk_s3::types::VersioningConfiguration::builder()
                .status(aws_sdk_s3::types::BucketVersioningStatus::Enabled)
                .build(),
        )
        .send()
        .await
        .expect("enable versioning");
}

#[tokio::test]
async fn delete_objects_by_prefix_spares_everything_outside_the_prefix() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");

    for key in [
        "records/a/one.txt",
        "records/a/two.txt",
        "records/a/nested/three.txt",
        "records/ab/keep.txt",
        "records/b/keep.txt",
        "clients/keep.json",
    ] {
        put(&s3, key).await;
    }

    let deleted = objects::delete_objects_by_prefix(&s3, BUCKET, "records/a/")
        .await
        .expect("delete by prefix");
    assert_eq!(deleted, 3);

    let mut remaining = objects::list_objects(&s3, BUCKET, "")
        .await
        .expect("list remaining");
    remaining.sort();
    assert_eq!(
        remaining,
        vec![
            "clients/keep.json".to_string(),
            "records/ab/keep.txt".to_string(),
            "records/b/keep.txt".to_string(),
        ]
    );
}

#[tokio::test]
async fn deletes_go_out_in_batches_of_a_thousand() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    create_versioned_bucket(&s3).await;
    // 2,350 versions: two full batches and a remainder, so the chunk boundary
    // is crossed twice.
    seed_versions(
        &server,
        (0..2350).map(|i| format!("records/obj-{i:05}.txt")),
    )
    .await;

    let deleted = objects::delete_all_object_versions(&s3, BUCKET, "")
        .await
        .expect("purge versions");
    assert_eq!(deleted, 2350);

    let batches = server.state.read().await.s3_delete_objects_batches.clone();
    assert_eq!(batches, vec![1000, 1000, 350]);

    assert!(server.state.read().await.objects.is_empty());
}

#[tokio::test]
async fn per_object_failure_inside_a_200_is_an_error_not_a_success() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    create_versioned_bucket(&s3).await;
    for key in ["records/one.txt", "records/two.txt", "records/locked.txt"] {
        put(&s3, key).await;
    }

    server
        .state
        .write()
        .await
        .s3_delete_object_failures
        .insert("records/locked.txt".to_string());

    let err = objects::delete_all_object_versions(&s3, BUCKET, "")
        .await
        .expect_err("a locked object must not report as deleted");

    match err {
        StorageError::DeleteObjects {
            attempted,
            failed,
            details,
        } => {
            assert_eq!(attempted, 3);
            assert_eq!(failed, 1);
            assert!(details.contains("records/locked.txt"), "got: {details}");
            assert!(details.contains("AccessDenied"), "got: {details}");
        }
        other => panic!("expected DeleteObjects, got {other:?}"),
    }
}

#[tokio::test]
async fn purging_versions_clears_history_and_delete_markers() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    create_versioned_bucket(&s3).await;

    // Three generations of one key, then a soft delete on top.
    for _ in 0..3 {
        put(&s3, "records/doc.txt").await;
    }
    objects::delete_object(&s3, BUCKET, "records/doc.txt")
        .await
        .expect("soft delete");
    put(&s3, "records/other.txt").await;

    let deleted = objects::delete_all_object_versions(&s3, BUCKET, "")
        .await
        .expect("purge versions");
    // Three versions plus the delete marker plus the untouched second key.
    assert_eq!(deleted, 5);

    assert!(server.state.read().await.objects.is_empty());
}

#[tokio::test]
async fn purging_versions_respects_the_prefix() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    create_versioned_bucket(&s3).await;
    put(&s3, "records/gone.txt").await;
    put(&s3, "records/gone.txt").await;
    put(&s3, "clients/kept.json").await;

    let deleted = objects::delete_all_object_versions(&s3, BUCKET, "records/")
        .await
        .expect("purge versions");
    assert_eq!(deleted, 2);

    let remaining = objects::list_objects(&s3, BUCKET, "")
        .await
        .expect("list remaining");
    assert_eq!(remaining, vec!["clients/kept.json".to_string()]);
}
