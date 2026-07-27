//! Bucket teardown against the in-process mock S3.
//!
//! Claria's own manifest turns versioning on, where deleting an object only
//! stacks a delete marker on top of it. A bucket "emptied" that way still
//! holds every version it ever had, and `DeleteBucket` answers BucketNotEmpty.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_sdk_s3::primitives::ByteStream;
use claria_mock_aws::testing::MockServer;
use claria_provisioner::{
    CredentialScope, Lifecycle, ProvisionerError, ResourceSpec, ResourceSyncer, Severity,
    syncers::s3_bucket::S3BucketSyncer,
};
use serde_json::json;

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

fn bucket_spec() -> ResourceSpec {
    ResourceSpec {
        resource_type: "s3_bucket".into(),
        resource_name: BUCKET.into(),
        lifecycle: Lifecycle::Managed,
        desired: json!({"region": "us-east-1"}),
        credential_scope: CredentialScope::Regular,
        label: "S3 Bucket".into(),
        description: "Encrypted storage for your client records and documents".into(),
        severity: Severity::Normal,
        iam_actions: vec![],
    }
}

/// A bucket shaped like Claria's: versioning on, several generations of a few
/// keys, and a couple of soft deletes leaving delete markers on top.
async fn seed_versioned_bucket(s3: &aws_sdk_s3::Client) {
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

    for key in [
        "clients/one.json",
        "records/one/note.txt",
        "records/one/scan.pdf",
        "_state/provisioner.json",
    ] {
        for _ in 0..3 {
            s3.put_object()
                .bucket(BUCKET)
                .key(key)
                .body(ByteStream::from_static(b"data"))
                .send()
                .await
                .expect("put object");
        }
    }

    // Soft-delete two of them: delete markers are versions too, and they keep
    // a bucket from being empty just as firmly as the data does.
    for key in ["records/one/note.txt", "clients/one.json"] {
        s3.delete_object()
            .bucket(BUCKET)
            .key(key)
            .send()
            .await
            .expect("soft delete");
    }
}

#[tokio::test]
async fn destroy_removes_a_versioned_bucket_with_history_and_delete_markers() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    seed_versioned_bucket(&s3).await;

    let syncer = S3BucketSyncer::new(bucket_spec(), s3.clone());
    syncer.destroy().await.expect("destroy a versioned bucket");

    let st = server.state.read().await;
    assert!(!st.buckets.contains_key(BUCKET), "bucket should be gone");
    assert!(st.objects.is_empty(), "no version should survive");
}

#[tokio::test]
async fn destroy_batches_its_deletes() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    seed_versioned_bucket(&s3).await;

    let syncer = S3BucketSyncer::new(bucket_spec(), s3.clone());
    syncer.destroy().await.expect("destroy");

    // 4 keys × 3 versions + 2 delete markers, in one request rather than 14.
    let batches = server.state.read().await.s3_delete_objects_batches.clone();
    assert_eq!(batches, vec![14]);
}

#[tokio::test]
async fn destroy_surfaces_an_object_that_would_not_delete() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    seed_versioned_bucket(&s3).await;
    server
        .state
        .write()
        .await
        .s3_delete_object_failures
        .insert("records/one/scan.pdf".to_string());

    let syncer = S3BucketSyncer::new(bucket_spec(), s3.clone());
    let err = syncer
        .destroy()
        .await
        .expect_err("an undeletable object must fail the destroy");

    match err {
        ProvisionerError::DeleteFailed(msg) => {
            assert!(msg.contains("records/one/scan.pdf"), "got: {msg}");
        }
        other => panic!("expected DeleteFailed, got {other:?}"),
    }

    // And the bucket is still standing rather than reported as destroyed.
    assert!(server.state.read().await.buckets.contains_key(BUCKET));
}

#[tokio::test]
async fn a_half_finished_destroy_can_be_re_run() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    seed_versioned_bucket(&s3).await;
    server
        .state
        .write()
        .await
        .s3_delete_object_failures
        .insert("records/one/scan.pdf".to_string());

    let syncer = S3BucketSyncer::new(bucket_spec(), s3.clone());
    syncer.destroy().await.expect_err("first attempt fails");

    // Whatever blocked the object clears; the operator hits Destroy again.
    server.state.write().await.s3_delete_object_failures.clear();

    syncer
        .destroy()
        .await
        .expect("re-run completes the teardown");

    let st = server.state.read().await;
    assert!(!st.buckets.contains_key(BUCKET));
    assert!(st.objects.is_empty());
}

#[tokio::test]
async fn destroying_an_already_destroyed_bucket_is_a_no_op() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    let syncer = S3BucketSyncer::new(bucket_spec(), s3.clone());
    syncer.destroy().await.expect("nothing to destroy");

    assert!(server.state.read().await.buckets.is_empty());
}
