use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_mock_aws::testing::MockServer;
use claria_report_store::template_library as writer_templates;
use claria_storage::client;
use uuid::Uuid;

const BUCKET: &str = "claria-writer-template-test";

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
    (server, s3)
}

#[tokio::test]
async fn templates_have_uuid_root_keys_and_managed_metadata() {
    let (_server, s3) = setup().await;
    let id = Uuid::new_v4();
    let source = b"mock docx bytes".to_vec();

    let created = writer_templates::create(&s3, BUCKET, id, "  Assessment shell  ", source.clone())
        .await
        .expect("create template");
    assert_eq!(created.metadata.id, id);
    assert_eq!(created.metadata.name, "Assessment shell");
    assert_eq!(created.metadata.size, source.len() as u64);
    assert_eq!(created.use_count, 0);

    let keys = claria_storage::objects::list_objects(
        &s3,
        BUCKET,
        claria_core::s3_keys::WRITER_TEMPLATES_PREFIX,
    )
    .await
    .expect("list objects");
    assert!(keys.contains(&format!("writer_templates/{id}.docx")));
    assert!(keys.contains(&format!("writer_templates/{id}.json")));
    assert!(keys.contains(&format!("writer_templates/{id}.usage.json")));

    let listed = writer_templates::list(&s3, BUCKET)
        .await
        .expect("list templates");
    assert_eq!(listed, vec![created.clone()]);
    assert_eq!(
        writer_templates::load_docx(&s3, BUCKET, id, 1_000)
            .await
            .expect("load docx"),
        source
    );

    let renamed = writer_templates::rename(&s3, BUCKET, id, "Updated shell")
        .await
        .expect("rename template");
    assert_eq!(renamed.metadata.name, "Updated shell");
    assert_eq!(
        writer_templates::increment_usage(&s3, BUCKET, id)
            .await
            .expect("increment usage"),
        1
    );
    assert_eq!(
        writer_templates::list(&s3, BUCKET)
            .await
            .expect("list used template")[0]
            .use_count,
        1
    );

    writer_templates::delete(&s3, BUCKET, id)
        .await
        .expect("delete template");
    assert!(
        writer_templates::list(&s3, BUCKET)
            .await
            .expect("list after delete")
            .is_empty()
    );
}

#[tokio::test]
async fn malformed_or_missing_usage_defaults_to_zero() {
    let (_server, s3) = setup().await;
    let id = Uuid::new_v4();
    writer_templates::create(&s3, BUCKET, id, "Template", vec![1, 2, 3])
        .await
        .expect("create template");

    claria_storage::objects::put_object(
        &s3,
        BUCKET,
        &claria_core::s3_keys::writer_template_usage(id),
        b"not json".to_vec(),
        Some("application/json"),
    )
    .await
    .expect("corrupt counter");
    assert_eq!(
        writer_templates::list(&s3, BUCKET)
            .await
            .expect("list with malformed usage")[0]
            .use_count,
        0
    );

    claria_storage::objects::delete_object(
        &s3,
        BUCKET,
        &claria_core::s3_keys::writer_template_usage(id),
    )
    .await
    .expect("delete counter");
    assert_eq!(
        writer_templates::list(&s3, BUCKET)
            .await
            .expect("list with missing usage")[0]
            .use_count,
        0
    );
}

#[test]
fn template_names_are_validated() {
    assert!(writer_templates::normalized_name("  ").is_err());
    assert!(writer_templates::normalized_name("bad\nname").is_err());
    assert!(writer_templates::normalized_name(&"x".repeat(121)).is_err());
    assert_eq!(
        writer_templates::normalized_name("  Useful template  ").expect("valid name"),
        "Useful template"
    );
}
