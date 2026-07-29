use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_client_lifecycle::{ClientLifecycleError, delete_client, restore_client};
use claria_core::models::{
    client::Client,
    report::{ReportContent, ReportWorkspace},
};
use claria_mock_aws::testing::MockServer;
use claria_storage::client;
use uuid::Uuid;

const BUCKET: &str = "claria-client-lifecycle-test";

fn sdk_config(endpoint: &str) -> aws_config::SdkConfig {
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
    let s3 = client::from_config(&sdk_config(&server.endpoint));
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
    (server, s3)
}

async fn put_json<T: serde::Serialize>(s3: &aws_sdk_s3::Client, key: &str, value: &T) {
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        key,
        serde_json::to_vec(value).expect("JSON"),
        Some("application/json"),
    )
    .await
    .expect("put JSON");
}

async fn put_text(s3: &aws_sdk_s3::Client, key: &str, value: &str) {
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        key,
        value.as_bytes().to_vec(),
        Some("text/plain"),
    )
    .await
    .expect("put text");
}

async fn seed_client(s3: &aws_sdk_s3::Client) -> (Uuid, String, String, String) {
    let client_id = Uuid::new_v4();
    let now = jiff::Timestamp::now();
    put_json(
        s3,
        &claria_core::s3_keys::client(client_id),
        &Client {
            id: client_id,
            name: "Lifecycle client".to_string(),
            created_at: now,
            updated_at: now,
        },
    )
    .await;
    let first = claria_core::s3_keys::client_record_file(client_id, "first.txt");
    let second = claria_core::s3_keys::client_record_file(client_id, "second.txt");
    put_text(s3, &first, "first record").await;
    put_text(s3, &second, "second record").await;
    let chat = claria_core::s3_keys::chat_history(client_id, Uuid::new_v4());
    put_text(s3, &chat, "existing text-only chat history").await;
    let workspace = claria_report_authoring::load_report_workspace(s3, BUCKET, client_id)
        .await
        .expect("workspace");
    claria_report_authoring::save_report_draft(
        s3,
        BUCKET,
        client_id,
        workspace.draft.revision,
        ReportContent {
            title: "Lifecycle report".to_string(),
            sections: vec![],
        },
    )
    .await
    .expect("save report");
    (client_id, first, second, chat)
}

async fn assert_current(s3: &aws_sdk_s3::Client, key: &str) {
    claria_storage::objects::get_object(s3, BUCKET, key)
        .await
        .expect("current object");
}

#[tokio::test]
async fn partial_child_deletion_is_compensated_before_the_client_remains_visible() {
    let (server, s3) = setup().await;
    let (client_id, first, second, chat) = seed_client(&s3).await;
    server
        .state
        .write()
        .await
        .s3_delete_object_failures
        .insert(second.clone());

    let error = delete_client(&s3, BUCKET, client_id)
        .await
        .expect_err("injected partial deletion");
    assert!(matches!(
        error,
        ClientLifecycleError::DeletionRolledBack { .. }
    ));
    assert_current(&s3, &claria_core::s3_keys::client(client_id)).await;
    assert_current(&s3, &first).await;
    assert_current(&s3, &second).await;
    assert_current(&s3, &chat).await;
    let workspace = claria_report_authoring::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("restored report");
    assert_eq!(workspace.draft.content.title, "Lifecycle report");
    assert!(matches!(
        claria_storage::objects::get_object(
            &s3,
            BUCKET,
            &claria_core::s3_keys::client_lifecycle(client_id)
        )
        .await,
        Err(claria_storage::error::StorageError::NotFound { .. })
    ));
}

#[tokio::test]
async fn restore_keeps_records_and_chat_deleted_without_report_rollback() {
    let (server, s3) = setup().await;
    let (client_id, first, second, chat) = seed_client(&s3).await;
    server.state.write().await.s3_delete_object_failures.clear();
    let outcome = delete_client(&s3, BUCKET, client_id)
        .await
        .expect("delete client");
    assert_eq!(outcome.deleted_records, 3);
    for key in [
        claria_core::s3_keys::client(client_id),
        first.clone(),
        second.clone(),
        chat.clone(),
        claria_core::s3_keys::report_workspace(client_id),
    ] {
        assert!(matches!(
            claria_storage::objects::get_object(&s3, BUCKET, &key).await,
            Err(claria_storage::error::StorageError::NotFound { .. })
        ));
    }

    let first_client = s3.clone();
    let second_client = s3.clone();
    let (first_restore, second_restore) = tokio::join!(
        restore_client(&first_client, BUCKET, client_id),
        restore_client(&second_client, BUCKET, client_id)
    );
    first_restore.expect("first restore");
    second_restore.expect("second restore");
    for key in [&first, &second, &chat] {
        assert!(matches!(
            claria_storage::objects::get_object(&s3, BUCKET, key).await,
            Err(claria_storage::error::StorageError::NotFound { .. })
        ));
    }

    let workspace = claria_report_authoring::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("restored report");
    let edited = claria_report_authoring::save_report_draft(
        &s3,
        BUCKET,
        client_id,
        workspace.draft.revision,
        ReportContent {
            title: "Edited after lifecycle restore".to_string(),
            sections: vec![],
        },
    )
    .await
    .expect("edit report");
    assert_eq!(edited.draft.revision, 2);

    restore_client(&s3, BUCKET, client_id)
        .await
        .expect("retried restore");
    let after_retry: ReportWorkspace =
        claria_report_authoring::load_report_workspace(&s3, BUCKET, client_id)
            .await
            .expect("load after retry");
    assert_eq!(after_retry.draft.revision, 2);
    assert_eq!(
        after_retry.draft.content.title,
        "Edited after lifecycle restore"
    );
}
