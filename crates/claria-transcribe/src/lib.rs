//! claria-transcribe
//!
//! Audio-to-text transcription via Amazon Transcribe.

pub mod error;

pub use aws_sdk_transcribe::types::MediaFormat;

use aws_sdk_transcribe::types::{Media, TranscriptionJobStatus};
use tracing::info;
use uuid::Uuid;

use crate::error::TranscribeError;

/// Transcribe an audio file already uploaded to S3.
///
/// Starts an Amazon Transcribe job pointing at the given S3 URI, directs the
/// output to the same bucket under `_transcribe/`, polls until completion,
/// reads the transcript JSON from S3, then cleans up the temporary output.
pub async fn transcribe_audio(
    config: &aws_config::SdkConfig,
    bucket: &str,
    audio_key: &str,
    media_format: MediaFormat,
) -> Result<String, TranscribeError> {
    let transcribe = aws_sdk_transcribe::Client::new(config);
    let s3 = aws_sdk_s3::Client::new(config);

    let job_name = format!("claria-{}", Uuid::new_v4());
    let s3_uri = format!("s3://{bucket}/{audio_key}");
    let output_key = format!("_transcribe/{job_name}.json");

    info!(job_name, s3_uri, "starting transcription job");

    transcribe
        .start_transcription_job()
        .transcription_job_name(&job_name)
        .media(Media::builder().media_file_uri(&s3_uri).build())
        .media_format(media_format)
        .language_code(aws_sdk_transcribe::types::LanguageCode::EnUs)
        .output_bucket_name(bucket)
        .output_key(&output_key)
        .send()
        .await
        .map_err(|e| TranscribeError::Api(e.into_service_error().to_string()))?;

    // Poll for completion.
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let resp = transcribe
            .get_transcription_job()
            .transcription_job_name(&job_name)
            .send()
            .await
            .map_err(|e| TranscribeError::Api(e.into_service_error().to_string()))?;

        let job = resp
            .transcription_job()
            .ok_or_else(|| TranscribeError::Api("no job in response".into()))?;

        match job.transcription_job_status() {
            Some(TranscriptionJobStatus::Completed) => break,
            Some(TranscriptionJobStatus::Failed) => {
                let reason = job.failure_reason().unwrap_or("unknown").to_string();
                let _ = transcribe
                    .delete_transcription_job()
                    .transcription_job_name(&job_name)
                    .send()
                    .await;
                return Err(TranscribeError::JobFailed(reason));
            }
            _ => continue,
        }
    }

    info!(job_name, "transcription complete, reading result from S3");

    // Read the transcript JSON from our bucket.
    let get_resp = s3
        .get_object()
        .bucket(bucket)
        .key(&output_key)
        .send()
        .await
        .map_err(|e| TranscribeError::Api(format!("failed to read transcript from S3: {e}")))?;

    let body = get_resp
        .body
        .collect()
        .await
        .map_err(|e| TranscribeError::Api(format!("failed to read transcript body: {e}")))?;

    let transcript_json = String::from_utf8(body.into_bytes().to_vec())
        .map_err(|e| TranscribeError::Parse(e.to_string()))?;

    let text = extract_transcript_text(&transcript_json)?;

    // Clean up: delete the temporary transcript JSON and the Transcribe job.
    let _ = s3
        .delete_object()
        .bucket(bucket)
        .key(&output_key)
        .send()
        .await;
    let _ = transcribe
        .delete_transcription_job()
        .transcription_job_name(&job_name)
        .send()
        .await;

    Ok(text)
}

/// Extract plain text from the Transcribe JSON response.
///
/// The response format is:
/// ```json
/// { "results": { "transcripts": [{ "transcript": "the text..." }] } }
/// ```
fn extract_transcript_text(json: &str) -> Result<String, TranscribeError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| TranscribeError::Parse(e.to_string()))?;

    let text = value
        .get("results")
        .and_then(|r| r.get("transcripts"))
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.first())
        .and_then(|t| t.get("transcript"))
        .and_then(|t| t.as_str())
        .unwrap_or("");

    Ok(text.to_string())
}

/// Map a file extension to an Amazon Transcribe `MediaFormat`.
///
/// Returns `None` for extensions that aren't supported audio formats.
pub fn media_format_for_extension(ext: &str) -> Option<MediaFormat> {
    match ext.to_lowercase().as_str() {
        "mp3" => Some(MediaFormat::Mp3),
        "mp4" | "m4a" => Some(MediaFormat::Mp4),
        "wav" => Some(MediaFormat::Wav),
        "flac" => Some(MediaFormat::Flac),
        "ogg" => Some(MediaFormat::Ogg),
        "amr" => Some(MediaFormat::Amr),
        "webm" => Some(MediaFormat::Webm),
        _ => None,
    }
}
