mod helpers;

use axum::http::{Method, StatusCode};

use helpers::{app, request_with_header};

async fn transcribe_request(app: &axum::Router, op: &str, json: &str) -> helpers::TestResponse {
    let target = format!("Transcribe.{op}");
    let body = json.to_string();
    request_with_header(app, Method::POST, "/", "x-amz-target", &target, body).await
}

#[tokio::test]
async fn start_and_get_transcription_job() {
    let app = app();

    // Need a bucket for output
    helpers::request(&app, Method::PUT, "/output-bucket", "").await;

    let r = transcribe_request(
        &app,
        "StartTranscriptionJob",
        r#"{
        "TranscriptionJobName": "test-job",
        "Media": {"MediaFileUri": "s3://input-bucket/audio.mp3"},
        "LanguageCode": "en-US",
        "OutputBucketName": "output-bucket",
        "OutputKey": "transcripts/test-job.json"
    }"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(body["TranscriptionJob"]["TranscriptionJobName"], "test-job");

    // GetTranscriptionJob should show COMPLETED
    let r = transcribe_request(
        &app,
        "GetTranscriptionJob",
        r#"{
        "TranscriptionJobName": "test-job"
    }"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(
        body["TranscriptionJob"]["TranscriptionJobStatus"],
        "COMPLETED"
    );

    // The transcript should have been written to S3
    let r = helpers::request(
        &app,
        Method::GET,
        "/output-bucket/transcripts/test-job.json",
        "",
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("transcript"));
}

#[tokio::test]
async fn get_nonexistent_job_returns_404() {
    let app = app();
    let r = transcribe_request(
        &app,
        "GetTranscriptionJob",
        r#"{
        "TranscriptionJobName": "nope"
    }"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_transcription_job() {
    let app = app();
    helpers::request(&app, Method::PUT, "/del-bucket", "").await;

    transcribe_request(
        &app,
        "StartTranscriptionJob",
        r#"{
        "TranscriptionJobName": "del-job",
        "Media": {"MediaFileUri": "s3://x/y"},
        "LanguageCode": "en-US",
        "OutputBucketName": "del-bucket",
        "OutputKey": "out.json"
    }"#,
    )
    .await;

    let r = transcribe_request(
        &app,
        "DeleteTranscriptionJob",
        r#"{
        "TranscriptionJobName": "del-job"
    }"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);

    let r = transcribe_request(
        &app,
        "GetTranscriptionJob",
        r#"{
        "TranscriptionJobName": "del-job"
    }"#,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}
