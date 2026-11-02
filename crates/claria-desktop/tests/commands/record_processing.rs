use super::*;
use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_mock_aws::{state::ScriptedBedrockResponse, testing::MockServer};

async fn setup() -> (MockServer, CommandContext) {
    let mock = MockServer::spawn().await;
    let sdk_config = aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(Credentials::new(
            "test",
            "test",
            None,
            None,
            "synthetic",
        )))
        .endpoint_url(&mock.endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build();
    let s3 = claria_storage::client::from_config(&sdk_config);
    let cfg = serde_json::from_value(serde_json::json!({
        "region":"us-east-1", "system_name":"test", "account_id":"123456789012",
        "created_at":"2026-01-01T00:00:00Z", "credentials":{"type":"default_chain"},
        "preferred_model_id":"chat-model", "extraction_model_id":"extraction-model"
    }))
    .unwrap();
    let ctx = CommandContext {
        cfg,
        sdk_config,
        s3,
        bucket: "test-bucket".into(),
    };
    ctx.s3
        .create_bucket()
        .bucket(&ctx.bucket)
        .send()
        .await
        .unwrap();
    (mock, ctx)
}

#[tokio::test]
async fn upload_and_reextract_use_explicit_model_selection_and_preserve_original() {
    let (mock, mut ctx) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.pdf");
    let original = b"%PDF-1.7\nsynthetic binary document";
    tokio::fs::write(&path, original).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let file = upload_record_file_inner(&ctx, &id.to_string(), path.to_str().unwrap(), None, false)
        .await
        .unwrap();
    assert_eq!(file.processing_status, Some(ProcessingStatus::Ready));
    let key = claria_core::s3_keys::client_record_file(id, &file.filename);
    ctx.cfg.extraction_model_id = Some("retry-model".into());
    assert_eq!(
        process_record(&ctx, id, &file.filename, "pdf", &key, Some(original), None)
            .await
            .unwrap(),
        ProcessingStatus::Ready
    );
    assert_eq!(
        claria_storage::objects::get_object(&ctx.s3, &ctx.bucket, &key)
            .await
            .unwrap()
            .body,
        original
    );
    assert_eq!(
        mock.state.read().await.bedrock_converse_model_ids,
        ["extraction-model", "retry-model"]
    );
}

#[tokio::test]
async fn provider_failure_returns_saved_original_and_actionable_reason_then_retry_recovers() {
    let (mock, ctx) = setup().await;
    mock.state.write().await.bedrock_text_responses.push(ScriptedBedrockResponse {
        status: 400, body: serde_json::json!({"__type":"ValidationException", "message":"private upstream detail"}),
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.pdf");
    let original = b"%PDF-1.7\nsynthetic";
    tokio::fs::write(&path, original).await.unwrap();
    let id = uuid::Uuid::new_v4();
    let file = upload_record_file_inner(&ctx, &id.to_string(), path.to_str().unwrap(), None, false)
        .await
        .unwrap();
    assert!(matches!(
        file.processing_status,
        Some(ProcessingStatus::Failed { .. })
    ));
    let key = claria_core::s3_keys::client_record_file(id, &file.filename);
    let marker = claria_storage::objects::get_object(&ctx.s3, &ctx.bucket, &format!("{key}.error"))
        .await
        .unwrap();
    assert!(
        !String::from_utf8(marker.body)
            .unwrap()
            .contains("private upstream")
    );
    assert_eq!(
        process_record(&ctx, id, &file.filename, "pdf", &key, Some(original), None)
            .await
            .unwrap(),
        ProcessingStatus::Ready
    );
    assert!(
        claria_records::processing::observe_error(&ctx.s3, &ctx.bucket, &key)
            .await
            .unwrap()
            .marker
            .is_none()
    );
    assert_eq!(
        claria_storage::objects::get_object(&ctx.s3, &ctx.bucket, &key)
            .await
            .unwrap()
            .body,
        original
    );
}

#[tokio::test]
async fn unconfigured_model_retains_original_and_does_not_invoke_provider() {
    let (mock, mut ctx) = setup().await;
    ctx.cfg.extraction_model_id = None;
    let id = uuid::Uuid::new_v4();
    let key = claria_core::s3_keys::client_record_file(id, "synthetic.pdf");
    let original = b"%PDF-1.7\nsynthetic";
    claria_storage::objects::put_object(&ctx.s3, &ctx.bucket, &key, original.to_vec(), None)
        .await
        .unwrap();
    assert!(matches!(
        process_record(&ctx, id, "synthetic.pdf", "pdf", &key, Some(original), None)
            .await
            .unwrap(),
        ProcessingStatus::Failed {
            code: ProcessingFailureCode::ModelNotConfigured,
            ..
        }
    ));
    assert!(
        mock.state
            .read()
            .await
            .bedrock_converse_model_ids
            .is_empty()
    );
}

#[tokio::test]
async fn account_scope_is_checked_before_processing() {
    let (_mock, ctx) = setup().await;
    assert!(check_processing_scope(&ctx, Some("another-account:us-east-1:test")).is_err());
    assert!(check_processing_scope(&ctx, Some("123456789012:us-east-1:test")).is_ok());
}

#[tokio::test]
async fn ordinary_error_collision_prevents_retry_and_upload_before_mutation() {
    let (mock, ctx) = setup().await;
    let id = uuid::Uuid::new_v4();
    let key = claria_core::s3_keys::client_record_file(id, "synthetic.pdf");
    for (key, body) in [
        (key.clone(), b"%PDF-1.7 original".to_vec()),
        (format!("{key}.text"), b"prior extraction".to_vec()),
        (format!("{key}.error"), b"ordinary user notes".to_vec()),
    ] {
        claria_storage::objects::put_object(&ctx.s3, &ctx.bucket, &key, body, None)
            .await
            .unwrap();
    }
    assert!(matches!(
        process_record(
            &ctx,
            id,
            "synthetic.pdf",
            "pdf",
            &key,
            Some(b"%PDF-1.7 original"),
            None
        )
        .await
        .unwrap(),
        ProcessingStatus::Failed {
            code: ProcessingFailureCode::ErrorFileConflict,
            ..
        }
    ));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.pdf");
    tokio::fs::write(&path, b"%PDF-1.7 replacement")
        .await
        .unwrap();
    assert!(
        upload_record_file_inner(&ctx, &id.to_string(), path.to_str().unwrap(), None, false)
            .await
            .is_err()
    );
    assert!(
        mock.state
            .read()
            .await
            .bedrock_converse_model_ids
            .is_empty()
    );
    for (key, expected) in [
        (key.clone(), b"%PDF-1.7 original".as_slice()),
        (format!("{key}.text"), b"prior extraction".as_slice()),
        (format!("{key}.error"), b"ordinary user notes".as_slice()),
    ] {
        assert_eq!(
            claria_storage::objects::get_object(&ctx.s3, &ctx.bucket, &key)
                .await
                .unwrap()
                .body,
            expected
        );
    }
}

#[tokio::test]
async fn printable_upload_ignores_user_error_collision_and_old_extraction() {
    let (_mock, ctx) = setup().await;
    let id = uuid::Uuid::new_v4();
    let key = claria_core::s3_keys::client_record_file(id, "synthetic.pdf");
    for (key, body) in [
        (format!("{key}.text"), b"old extraction".to_vec()),
        (format!("{key}.error"), b"ordinary user note".to_vec()),
    ] {
        claria_storage::objects::put_object(&ctx.s3, &ctx.bucket, &key, body, None)
            .await
            .unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.pdf");
    tokio::fs::write(&path, b"current printable original")
        .await
        .unwrap();
    let file = upload_record_file_inner(&ctx, &id.to_string(), path.to_str().unwrap(), None, false)
        .await
        .unwrap();
    assert_eq!(file.processing_status, Some(ProcessingStatus::Ready));
    assert_eq!(
        claria_records::fetch_record_text(
            &ctx.s3,
            &ctx.bucket,
            id,
            "synthetic.pdf",
            &claria_records::RecordCache::new()
        )
        .await
        .unwrap()
        .as_deref(),
        Some("current printable original")
    );
    assert_eq!(
        claria_storage::objects::get_object(&ctx.s3, &ctx.bucket, &format!("{key}.error"))
            .await
            .unwrap()
            .body,
        b"ordinary user note"
    );
}
