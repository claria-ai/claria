use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_mock_aws::testing::MockServer;
use claria_storage::{client, error::StorageError, state};
use serde::{Deserialize, Serialize};

const BUCKET: &str = "claria-state-locking-test";
const KEY: &str = "report-authoring/client/workspace.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct TestState {
    revision: u64,
}

fn build_sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let credentials = Credentials::new("test", "test", None, None, "test");
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

async fn setup() -> (MockServer, aws_sdk_s3::Client) {
    let server = MockServer::spawn().await;
    let sdk_config = build_sdk_config(&server.endpoint);
    let s3 = client::from_config(&sdk_config);
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");
    (server, s3)
}

#[tokio::test]
async fn concurrent_conditional_create_succeeds_exactly_once() {
    let (_server, s3) = setup().await;
    let first_client = s3.clone();
    let second_client = s3.clone();
    let first_state = TestState { revision: 1 };
    let second_state = TestState { revision: 2 };

    let (first, second) = tokio::join!(
        state::save_state_if_none_match(&first_client, BUCKET, KEY, &first_state),
        state::save_state_if_none_match(&second_client, BUCKET, KEY, &second_state)
    );
    assert_ne!(first.is_ok(), second.is_ok());
    let loser = if first.is_err() { first } else { second };
    assert!(matches!(
        loser.expect_err("one loser"),
        StorageError::PreconditionFailed { .. }
    ));

    let (loaded, loaded_etag): (TestState, String) =
        state::load_state(&s3, BUCKET, KEY).await.expect("load");
    assert!(loaded == first_state || loaded == second_state);
    assert!(!loaded_etag.is_empty());
}

#[tokio::test]
async fn conditional_conflicts_are_retried_and_then_classified() {
    let (server, s3) = setup().await;
    server
        .state
        .write()
        .await
        .s3_conditional_conflicts_remaining
        .insert(KEY.to_string(), 2);
    state::save_state_if_none_match(&s3, BUCKET, KEY, &TestState { revision: 0 })
        .await
        .expect("transient 409s are retried");

    let another_key = format!("{KEY}.another");
    server
        .state
        .write()
        .await
        .s3_conditional_conflicts_remaining
        .insert(another_key.clone(), 4);
    let error =
        state::save_state_if_none_match(&s3, BUCKET, &another_key, &TestState { revision: 0 })
            .await
            .expect_err("persistent 409");
    assert!(matches!(
        error,
        StorageError::ConditionalRequestConflict { .. }
    ));
}

#[tokio::test]
async fn bounded_get_rejects_oversized_objects_before_collection() {
    let (server, s3) = setup().await;
    claria_storage::objects::put_object(
        &s3,
        BUCKET,
        "large.txt",
        vec![b'x'; 1024],
        Some("text/plain"),
    )
    .await
    .expect("put large object");
    let error = claria_storage::objects::get_object_bounded(&s3, BUCKET, "large.txt", 100)
        .await
        .expect_err("size ceiling");
    assert!(matches!(error, StorageError::ObjectTooLarge { .. }));
    assert_eq!(
        server
            .state
            .read()
            .await
            .s3_get_object_requests
            .get("large.txt"),
        Some(&1)
    );
}

#[tokio::test]
async fn matching_etag_updates_and_stale_etag_cannot_overwrite() {
    let (_server, s3) = setup().await;
    let etag = state::save_state_if_none_match(&s3, BUCKET, KEY, &TestState { revision: 0 })
        .await
        .expect("create");

    let new_etag = state::save_state_if_match(&s3, BUCKET, KEY, &TestState { revision: 1 }, &etag)
        .await
        .expect("matching update");
    assert_ne!(new_etag, etag);

    let error = state::save_state_if_match(&s3, BUCKET, KEY, &TestState { revision: 2 }, &etag)
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::PreconditionFailed { .. }));

    let (loaded, _): (TestState, String) = state::load_state(&s3, BUCKET, KEY).await.expect("load");
    assert_eq!(loaded.revision, 1);
}
