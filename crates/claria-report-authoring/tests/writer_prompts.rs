use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_mock_aws::testing::MockServer;
use claria_report_authoring::writer_prompts;
use claria_storage::client;
use uuid::Uuid;

const BUCKET: &str = "claria-writer-prompt-test";

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
async fn prompts_round_trip_under_the_library_prefix_sorted_by_name() {
    let (_server, s3) = setup().await;
    let phase_two = Uuid::new_v4();
    let phase_one = Uuid::new_v4();

    let created = writer_prompts::create(
        &s3,
        BUCKET,
        phase_two,
        "  Phase 2 — summary  ",
        "  Draft a summary and clinical interpretation backing my diagnosis of $DIAGNOSIS.  ",
    )
    .await
    .expect("create second-phase prompt");
    assert_eq!(created.id, phase_two);
    assert_eq!(created.name, "Phase 2 — summary");
    assert_eq!(
        created.body,
        "Draft a summary and clinical interpretation backing my diagnosis of $DIAGNOSIS."
    );

    writer_prompts::create(
        &s3,
        BUCKET,
        phase_one,
        "Phase 1 — history",
        "Fill in Reason for Referral, Background, and Medical/Social History; skip everything else.",
    )
    .await
    .expect("create first-phase prompt");

    let keys = claria_storage::objects::list_objects(
        &s3,
        BUCKET,
        claria_core::s3_keys::WRITER_PROMPT_LIBRARY_PREFIX,
    )
    .await
    .expect("list objects");
    assert!(keys.contains(&format!("claria-prompts/writer-library/{phase_one}.json")));
    assert!(keys.contains(&format!("claria-prompts/writer-library/{phase_two}.json")));

    let listed = writer_prompts::list(&s3, BUCKET).await.expect("list");
    assert_eq!(
        listed.iter().map(|prompt| prompt.id).collect::<Vec<_>>(),
        vec![phase_one, phase_two],
        "alphabetical by name, not creation order"
    );

    let updated = writer_prompts::update(
        &s3,
        BUCKET,
        phase_two,
        "Phase 2 — interpretation",
        "Draft the Summary and Clinical Interpretation sections backing my diagnosis of $DIAGNOSIS.",
    )
    .await
    .expect("update prompt");
    assert_eq!(updated.created_at, created.created_at);
    assert!(updated.updated_at >= created.updated_at);

    writer_prompts::delete(&s3, BUCKET, phase_one)
        .await
        .expect("delete prompt");
    let remaining = writer_prompts::list(&s3, BUCKET)
        .await
        .expect("list after delete");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].name, "Phase 2 — interpretation");
}

#[tokio::test]
async fn prompt_validation_rejects_empty_oversized_and_mismatched_input() {
    let (_server, s3) = setup().await;

    let empty_name = writer_prompts::create(&s3, BUCKET, Uuid::new_v4(), "   ", "body")
        .await
        .expect_err("empty name");
    assert!(empty_name.to_string().contains("Enter a name"));

    let empty_body = writer_prompts::create(&s3, BUCKET, Uuid::new_v4(), "Name", " \n ")
        .await
        .expect_err("empty body");
    assert!(empty_body.to_string().contains("Enter the prompt text"));

    let long_body = "x".repeat(writer_prompts::MAX_WRITER_PROMPT_BODY_CHARACTERS + 1);
    let oversized = writer_prompts::create(&s3, BUCKET, Uuid::new_v4(), "Name", &long_body)
        .await
        .expect_err("oversized body");
    assert!(oversized.to_string().contains("at most"));

    let control_name = writer_prompts::create(&s3, BUCKET, Uuid::new_v4(), "bad\nname", "body")
        .await
        .expect_err("control characters in name");
    assert!(control_name.to_string().contains("control characters"));

    // Multi-line bodies are ordinary instructions and must be preserved.
    let multiline = writer_prompts::create(
        &s3,
        BUCKET,
        Uuid::new_v4(),
        "Phased instruction",
        "Line one.\nLine two.",
    )
    .await
    .expect("multi-line body");
    assert_eq!(multiline.body, "Line one.\nLine two.");

    let missing = writer_prompts::update(&s3, BUCKET, Uuid::new_v4(), "Name", "body")
        .await
        .expect_err("update of a missing prompt");
    assert!(!missing.to_string().is_empty());
}
