use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use serde_json::{json, Value};

use crate::state::{ObjectVersion, SharedState, TranscriptionJob};

/// Dispatch Transcribe JSON-protocol requests.
pub async fn dispatch(target_suffix: &str, body: Value, state: SharedState) -> Response {
    match target_suffix {
        "StartTranscriptionJob" => start_job(body, state).await,
        "GetTranscriptionJob" => get_job(body, state).await,
        "DeleteTranscriptionJob" => delete_job(body, state).await,
        _ => (
            StatusCode::BAD_REQUEST,
            json!({"__type": "InvalidAction", "message": format!("Unknown Transcribe action: {target_suffix}")}).to_string(),
        ).into_response(),
    }
}

fn json_response(value: Value) -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/x-amz-json-1.1")],
        value.to_string(),
    )
        .into_response()
}

async fn start_job(body: Value, state: SharedState) -> Response {
    let job_name = body["TranscriptionJobName"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let media_uri = body["Media"]["MediaFileUri"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let language = body["LanguageCode"]
        .as_str()
        .unwrap_or("en-US")
        .to_string();
    let output_bucket = body["OutputBucketName"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let output_key = body["OutputKey"]
        .as_str()
        .unwrap_or(&format!("_transcribe/{job_name}.json"))
        .to_string();

    let mut st = state.write().await;

    // Write the mock transcript result to S3 state immediately
    let transcript_json = json!({
        "results": {
            "transcripts": [{
                "transcript": "This is a mock transcription of the audio file. \
                    The patient discussed their symptoms and treatment progress."
            }]
        }
    });

    let transcript_bytes = Bytes::from(transcript_json.to_string());
    let now = jiff::Timestamp::now().to_string();
    let versions = st
        .objects
        .entry((output_bucket.clone(), output_key.clone()))
        .or_default();
    versions.clear();
    versions.push(ObjectVersion {
        version_id: "null".to_string(),
        body: transcript_bytes,
        content_type: "application/json".to_string(),
        etag: "mock-transcript-etag".to_string(),
        last_modified: now,
        is_delete_marker: false,
    });

    let job = TranscriptionJob {
        job_name: job_name.clone(),
        status: "COMPLETED".to_string(),
        media_uri,
        output_bucket: output_bucket.clone(),
        output_key: output_key.clone(),
        language_code: language.clone(),
    };

    st.transcription_jobs.insert(job_name.clone(), job);

    json_response(json!({
        "TranscriptionJob": {
            "TranscriptionJobName": job_name,
            "TranscriptionJobStatus": "IN_PROGRESS",
            "LanguageCode": language,
        }
    }))
}

async fn get_job(body: Value, state: SharedState) -> Response {
    let job_name = body["TranscriptionJobName"]
        .as_str()
        .unwrap_or("");
    let st = state.read().await;

    match st.transcription_jobs.get(job_name) {
        Some(job) => {
            let output_uri = format!(
                "s3://{}/{}",
                job.output_bucket, job.output_key
            );
            json_response(json!({
                "TranscriptionJob": {
                    "TranscriptionJobName": job.job_name,
                    "TranscriptionJobStatus": job.status,
                    "LanguageCode": job.language_code,
                    "Transcript": {
                        "TranscriptFileUri": output_uri,
                    }
                }
            }))
        }
        None => (
            StatusCode::NOT_FOUND,
            json!({"__type": "BadRequestException", "message": "Job not found"}).to_string(),
        )
            .into_response(),
    }
}

async fn delete_job(body: Value, state: SharedState) -> Response {
    let job_name = body["TranscriptionJobName"]
        .as_str()
        .unwrap_or("");
    let mut st = state.write().await;
    st.transcription_jobs.remove(job_name);
    json_response(json!({}))
}
