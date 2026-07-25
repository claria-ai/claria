//! claria-transcribe
//!
//! Audio-to-text transcription via Amazon Transcribe (standard and medical).
//!
//! Two APIs:
//!
//! - [`transcribe_audio_with_options`] — full-featured, accepts [`TranscribeOptions`]
//!   (language mode, speaker handling, engine, custom vocabulary) and returns a
//!   structured [`TranscriptResult`] preserving speaker turns, timestamps, and
//!   per-segment language codes.
//! - [`transcribe_audio`] — thin legacy wrapper that hardcodes English + standard
//!   engine + 2-speaker diarization and returns the rendered plain-text body.
//!   Used by the existing drag-drop upload path during the transition window.
//!
//! The structured result is rendered to and parsed from a lightly-structured
//! plain-text-with-headers format via [`format_transcript_body`] and
//! [`parse_transcript_body`]. The same `.text` sidecar in S3 holds the body;
//! S3 object versioning gives us v1-as-canonical history for free.

pub mod error;

pub use aws_sdk_transcribe::types::MediaFormat;
pub use error::TranscribeError;

use aws_sdk_transcribe::types::{
    LanguageCode, Media, MedicalContentIdentificationType, MedicalTranscriptionSetting, Settings,
    Specialty, TranscriptionJobStatus, Type as MedicalType,
};
use backon::{ExponentialBuilder, Retryable};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::Duration;
use tracing::info;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public options
// ---------------------------------------------------------------------------

/// Caller-facing transcription options.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscribeOptions {
    pub language: LanguageMode,
    pub speakers: SpeakerHandling,
    pub engine: TranscriptionEngine,
    // TODO(vocab): custom vocabularies are a separate AWS resource type per
    // (engine, language) — Standard en-US, Standard es-US, and Medical en-US
    // are three different resources via CreateVocabulary / CreateMedicalVocabulary,
    // and a single string can't validly serve all three. Claria does not yet
    // manage vocabularies (no create / list / validate flow) so plumbing the
    // name through here without that infrastructure silently fails for Mixed
    // jobs and is brittle for Medical. Re-add as a typed shape
    // (standard_english / standard_spanish / medical_english) plus a
    // "Manage vocabularies" UI when there's user demand.
}

impl Default for TranscribeOptions {
    fn default() -> Self {
        Self {
            language: LanguageMode::English,
            speakers: SpeakerHandling::Diarize { max: 2 },
            engine: TranscriptionEngine::Standard,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LanguageMode {
    /// English only (en-US).
    English,
    /// Spanish only (es-US).
    Spanish,
    /// Code-switching session with English and Spanish interleaved.
    /// Drives `IdentifyMultipleLanguages` with `LanguageOptions=[en-US, es-US]`.
    Mixed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SpeakerHandling {
    /// Single speaker; no diarization, no channel ID.
    None,
    /// Speaker diarization with the given max (clamped to 2..=10 by the API).
    Diarize { max: u8 },
    /// Two-channel audio (e.g. clinician on L, patient on R). Mutually exclusive with diarization.
    Channels,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionEngine {
    /// Amazon Transcribe (general purpose).
    Standard,
    /// Amazon Transcribe Medical. English (en-US) only.
    Medical,
}

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

/// Structured result of a transcription job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptResult {
    pub segments: Vec<TranscriptSegment>,
    pub speakers: Vec<Speaker>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptSegment {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_code: Option<String>,
    pub start_seconds: u32,
    pub end_seconds: u32,
    pub text: String,
    /// English translation of `text`, populated when the user has translation
    /// enabled and `language_code` is not `en-US`. Rendered in the body as
    /// `> `-prefixed lines beneath the original.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Speaker {
    pub id: String,
    pub label: String,
}

// ---------------------------------------------------------------------------
// Engine resolution
// ---------------------------------------------------------------------------

/// Resolve a caller-requested engine against the chosen language.
///
/// Medical only supports English. If the caller asked for Medical with a
/// non-English language, we transparently fall back to Standard — the wizard's
/// "engine is auto-routed from language" contract.
fn resolve_engine(requested: TranscriptionEngine, language: LanguageMode) -> TranscriptionEngine {
    match (requested, language) {
        (TranscriptionEngine::Medical, LanguageMode::English) => TranscriptionEngine::Medical,
        (TranscriptionEngine::Medical, _) => TranscriptionEngine::Standard,
        (TranscriptionEngine::Standard, _) => TranscriptionEngine::Standard,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Transcribe an audio file already uploaded to S3, with the legacy fixed
/// options (English, standard engine, 2-speaker diarization). Returns the
/// rendered plain-text-with-headers body suitable for direct write to the
/// `.text` sidecar.
///
/// Equivalent to calling [`transcribe_audio_with_options`] with
/// [`TranscribeOptions::default`] and rendering the result via
/// [`format_transcript_body`].
pub async fn transcribe_audio(
    config: &aws_config::SdkConfig,
    bucket: &str,
    audio_key: &str,
    media_format: MediaFormat,
) -> Result<String, TranscribeError> {
    let result =
        transcribe_audio_with_options(config, bucket, audio_key, media_format, &Default::default())
            .await?;
    Ok(format_transcript_body(&result))
}

/// Transcribe an audio file already uploaded to S3, with caller-controlled
/// options. Returns the structured [`TranscriptResult`].
pub async fn transcribe_audio_with_options(
    config: &aws_config::SdkConfig,
    bucket: &str,
    audio_key: &str,
    media_format: MediaFormat,
    options: &TranscribeOptions,
) -> Result<TranscriptResult, TranscribeError> {
    let transcribe = aws_sdk_transcribe::Client::new(config);
    let s3 = claria_storage::client::from_config(config);

    let job_name = format!("claria-{}", Uuid::new_v4());
    let s3_uri = format!("s3://{bucket}/{audio_key}");
    let output_key = format!("_transcribe/{job_name}.json");
    let engine = resolve_engine(options.engine, options.language);

    info!(
        job_name,
        s3_uri,
        engine = ?engine,
        language = ?options.language,
        "starting transcription job"
    );

    match engine {
        TranscriptionEngine::Standard => {
            start_standard_job(
                &transcribe,
                &job_name,
                &s3_uri,
                media_format,
                bucket,
                &output_key,
                options,
            )
            .await?;
            poll_standard_job(&transcribe, &job_name).await?;
        }
        TranscriptionEngine::Medical => {
            start_medical_job(
                &transcribe,
                &job_name,
                &s3_uri,
                media_format,
                bucket,
                &output_key,
                options,
            )
            .await?;
            poll_medical_job(&transcribe, &job_name).await?;
        }
    }

    info!(job_name, "transcription complete, reading result from S3");

    let transcript_json = claria_storage::objects::get_object(&s3, bucket, &output_key)
        .await
        .map_err(|e| {
            tracing::error!(
                job_name, bucket, output_key = %output_key, error = %e,
                "failed to read transcript result from S3"
            );
            TranscribeError::Api(format!("failed to read transcript from S3: {e}"))
        })?;

    let body_str = String::from_utf8(transcript_json.body).map_err(|e| {
        tracing::error!(job_name, error = %e, "transcript result from S3 is not valid UTF-8");
        TranscribeError::Parse(e.to_string())
    })?;

    let result = parse_transcribe_json(&body_str)?;

    // Clean up temporary job + intermediate JSON.
    let _ = claria_storage::objects::delete_object(&s3, bucket, &output_key).await;
    delete_job(&transcribe, &job_name, engine).await;

    Ok(result)
}

// ---------------------------------------------------------------------------
// Job polling
// ---------------------------------------------------------------------------

/// Longest we will wait for a transcription job before giving up on it.
///
/// AWS Transcribe routinely takes several minutes on long recordings — the
/// job runs roughly in proportion to the media length — so this ceiling has
/// to be generous or it would abort real transcriptions. Thirty minutes is
/// far beyond any recording Claria uploads today, and it guarantees the
/// calling Tauri command eventually returns instead of freezing the UI on a
/// job AWS has silently wedged.
const POLL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Shortest gap between status polls, matching the previous fixed cadence.
const POLL_MIN_DELAY: Duration = Duration::from_secs(3);

/// Longest gap between status polls. Long jobs don't need second-by-second
/// attention, and backing off keeps the GetTranscriptionJob call count down.
const POLL_MAX_DELAY: Duration = Duration::from_secs(30);

/// Why a poll attempt didn't produce a finished job.
///
/// `Pending` is the retryable case; `Fatal` stops the loop immediately.
enum PollError {
    Pending,
    Fatal(TranscribeError),
}

/// Jittered exponential backoff bounded by [`POLL_TIMEOUT`].
///
/// Jitter matters because several uploads can finish together and would
/// otherwise poll AWS in lockstep.
fn poll_backoff() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(POLL_MIN_DELAY)
        .with_max_delay(POLL_MAX_DELAY)
        .with_factor(1.5)
        .with_jitter()
        .without_max_times()
        .with_total_delay(Some(POLL_TIMEOUT))
}

/// Turn a poll outcome into a `TranscribeError`, deleting the job in AWS on
/// any terminal failure so a wedged job doesn't linger in the account.
async fn finish_poll(
    transcribe: &aws_sdk_transcribe::Client,
    job_name: &str,
    outcome: Result<(), PollError>,
    engine: TranscriptionEngine,
) -> Result<(), TranscribeError> {
    let err = match outcome {
        Ok(()) => return Ok(()),
        Err(PollError::Fatal(TranscribeError::JobFailed(reason))) => {
            tracing::error!(job_name, reason = %reason, engine = ?engine, "transcription job failed");
            TranscribeError::JobFailed(reason)
        }
        Err(PollError::Fatal(e)) => return Err(e),
        Err(PollError::Pending) => {
            let minutes = POLL_TIMEOUT.as_secs() / 60;
            tracing::error!(job_name, minutes, engine = ?engine, "transcription job timed out");
            TranscribeError::Timeout {
                job_name: job_name.to_string(),
                minutes,
            }
        }
    };

    delete_job(transcribe, job_name, engine).await;
    Err(err)
}

/// Best-effort cleanup of a transcription job. Failures here are not worth
/// surfacing — the caller already has a real error to report.
async fn delete_job(
    transcribe: &aws_sdk_transcribe::Client,
    job_name: &str,
    engine: TranscriptionEngine,
) {
    match engine {
        TranscriptionEngine::Standard => {
            let _ = transcribe
                .delete_transcription_job()
                .transcription_job_name(job_name)
                .send()
                .await;
        }
        TranscriptionEngine::Medical => {
            let _ = transcribe
                .delete_medical_transcription_job()
                .medical_transcription_job_name(job_name)
                .send()
                .await;
        }
    }
}

// ---------------------------------------------------------------------------
// Standard engine
// ---------------------------------------------------------------------------

async fn start_standard_job(
    transcribe: &aws_sdk_transcribe::Client,
    job_name: &str,
    s3_uri: &str,
    media_format: MediaFormat,
    output_bucket: &str,
    output_key: &str,
    options: &TranscribeOptions,
) -> Result<(), TranscribeError> {
    let mut req = transcribe
        .start_transcription_job()
        .transcription_job_name(job_name)
        .media(Media::builder().media_file_uri(s3_uri).build())
        .media_format(media_format)
        .output_bucket_name(output_bucket)
        .output_key(output_key);

    req = match options.language {
        LanguageMode::English => req.language_code(LanguageCode::EnUs),
        LanguageMode::Spanish => req.language_code(LanguageCode::EsUs),
        LanguageMode::Mixed => req
            .identify_multiple_languages(true)
            .language_options(LanguageCode::EnUs)
            .language_options(LanguageCode::EsUs),
        // TODO(vocab): when re-introducing vocabularies, Mixed mode needs
        // per-language vocab names via LanguageIdSettings (one entry per
        // language code, each pointing at a vocabulary registered for that
        // language). A single shared vocab is silently wrong — AWS will
        // either skip or fail whichever language doesn't match.
    };

    if let Some(settings) = build_standard_settings(options) {
        req = req.settings(settings);
    }

    req.send()
        .await
        .map_err(|e| TranscribeError::Api(e.into_service_error().to_string()))?;
    Ok(())
}

fn build_standard_settings(options: &TranscribeOptions) -> Option<Settings> {
    let mut builder = Settings::builder();
    let mut touched = false;

    match options.speakers {
        SpeakerHandling::None => {}
        SpeakerHandling::Diarize { max } => {
            let clamped = max.clamp(2, 10) as i32;
            builder = builder
                .show_speaker_labels(true)
                .max_speaker_labels(clamped);
            touched = true;
        }
        SpeakerHandling::Channels => {
            builder = builder.channel_identification(true);
            touched = true;
        }
    }

    // TODO(vocab): Settings::vocabulary_name goes here for the non-Mixed case
    // once Claria has a vocabulary-management surface.

    if touched { Some(builder.build()) } else { None }
}

async fn poll_standard_job(
    transcribe: &aws_sdk_transcribe::Client,
    job_name: &str,
) -> Result<(), TranscribeError> {
    let outcome = (|| async {
        let resp = transcribe
            .get_transcription_job()
            .transcription_job_name(job_name)
            .send()
            .await
            .map_err(|e| {
                PollError::Fatal(TranscribeError::Api(e.into_service_error().to_string()))
            })?;

        let job = resp.transcription_job().ok_or_else(|| {
            PollError::Fatal(TranscribeError::Api("no job in response".into()))
        })?;

        match job.transcription_job_status() {
            Some(TranscriptionJobStatus::Completed) => Ok(()),
            Some(TranscriptionJobStatus::Failed) => Err(PollError::Fatal(
                TranscribeError::JobFailed(job.failure_reason().unwrap_or("unknown").to_string()),
            )),
            _ => Err(PollError::Pending),
        }
    })
    .retry(poll_backoff())
    .when(|e| matches!(e, PollError::Pending))
    .notify(|_, delay| tracing::debug!(job_name, ?delay, "transcription job still running"))
    .await;

    finish_poll(transcribe, job_name, outcome, TranscriptionEngine::Standard).await
}

// ---------------------------------------------------------------------------
// Medical engine
// ---------------------------------------------------------------------------

async fn start_medical_job(
    transcribe: &aws_sdk_transcribe::Client,
    job_name: &str,
    s3_uri: &str,
    media_format: MediaFormat,
    output_bucket: &str,
    output_key: &str,
    options: &TranscribeOptions,
) -> Result<(), TranscribeError> {
    if !matches!(options.language, LanguageMode::English) {
        return Err(TranscribeError::MedicalUnsupportedLanguage(format!(
            "{:?}",
            options.language
        )));
    }

    let mut req = transcribe
        .start_medical_transcription_job()
        .medical_transcription_job_name(job_name)
        .media(Media::builder().media_file_uri(s3_uri).build())
        .media_format(media_format)
        .language_code(LanguageCode::EnUs)
        .output_bucket_name(output_bucket)
        .output_key(output_key)
        .specialty(Specialty::Primarycare)
        .r#type(MedicalType::Conversation)
        .content_identification_type(MedicalContentIdentificationType::Phi);

    if let Some(settings) = build_medical_settings(options) {
        req = req.settings(settings);
    }

    req.send()
        .await
        .map_err(|e| TranscribeError::Api(e.into_service_error().to_string()))?;
    Ok(())
}

fn build_medical_settings(options: &TranscribeOptions) -> Option<MedicalTranscriptionSetting> {
    let mut builder = MedicalTranscriptionSetting::builder();
    let mut touched = false;

    match options.speakers {
        SpeakerHandling::None => {}
        SpeakerHandling::Diarize { max } => {
            let clamped = max.clamp(2, 10) as i32;
            builder = builder
                .show_speaker_labels(true)
                .max_speaker_labels(clamped);
            touched = true;
        }
        SpeakerHandling::Channels => {
            builder = builder.channel_identification(true);
            touched = true;
        }
    }

    // TODO(vocab): MedicalTranscriptionSetting::vocabulary_name goes here once
    // Claria has a medical-vocabulary management surface. Note Medical vocabs
    // are a separate AWS resource type (CreateMedicalVocabulary), distinct
    // from standard vocabs — the namespaces don't overlap.

    if touched { Some(builder.build()) } else { None }
}

async fn poll_medical_job(
    transcribe: &aws_sdk_transcribe::Client,
    job_name: &str,
) -> Result<(), TranscribeError> {
    let outcome = (|| async {
        let resp = transcribe
            .get_medical_transcription_job()
            .medical_transcription_job_name(job_name)
            .send()
            .await
            .map_err(|e| {
                PollError::Fatal(TranscribeError::Api(e.into_service_error().to_string()))
            })?;

        let job = resp.medical_transcription_job().ok_or_else(|| {
            PollError::Fatal(TranscribeError::Api("no medical job in response".into()))
        })?;

        match job.transcription_job_status() {
            Some(TranscriptionJobStatus::Completed) => Ok(()),
            Some(TranscriptionJobStatus::Failed) => Err(PollError::Fatal(
                TranscribeError::JobFailed(job.failure_reason().unwrap_or("unknown").to_string()),
            )),
            _ => Err(PollError::Pending),
        }
    })
    .retry(poll_backoff())
    .when(|e| matches!(e, PollError::Pending))
    .notify(
        |_, delay| tracing::debug!(job_name, ?delay, "medical transcription job still running"),
    )
    .await;

    finish_poll(transcribe, job_name, outcome, TranscriptionEngine::Medical).await
}

// ---------------------------------------------------------------------------
// Transcribe JSON → structured TranscriptResult
// ---------------------------------------------------------------------------

/// Parse Amazon Transcribe's output JSON (works for both standard and medical
/// responses) into a structured [`TranscriptResult`].
pub fn parse_transcribe_json(json: &str) -> Result<TranscriptResult, TranscribeError> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| {
        tracing::error!(error = %e, "failed to parse Transcribe output JSON");
        TranscribeError::Parse(e.to_string())
    })?;

    let results = value
        .get("results")
        .ok_or_else(|| TranscribeError::Parse("missing `results` block".into()))?;

    let items = results.get("items").and_then(|i| i.as_array());
    let speaker_labels = results.get("speaker_labels");

    let speakers = extract_speakers(speaker_labels);

    let mut segments = if let (Some(items), Some(labels)) =
        (items, speaker_labels.and_then(|s| s.get("segments")))
    {
        segments_by_speaker(items, labels)
    } else if let Some(items) = items {
        segments_by_language(items)
    } else {
        let text = results
            .get("transcripts")
            .and_then(|t| t.as_array())
            .and_then(|a| a.first())
            .and_then(|t| t.get("transcript"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        vec![TranscriptSegment {
            id: "seg_0001".into(),
            speaker_id: None,
            language_code: None,
            start_seconds: 0,
            end_seconds: 0,
            text,
            translation: None,
        }]
    };

    // AWS returns speaker_labels.segments grouped by speaker, not in time
    // order. Sort here so downstream (body render, UI, edit save/load) sees a
    // chronological transcript. Reassign IDs post-sort so seg_0001 is always
    // the first thing said.
    segments.sort_by_key(|s| (s.start_seconds, s.end_seconds));
    for (i, seg) in segments.iter_mut().enumerate() {
        seg.id = format!("seg_{:04}", i + 1);
    }

    Ok(TranscriptResult { segments, speakers })
}

fn extract_speakers(speaker_labels: Option<&serde_json::Value>) -> Vec<Speaker> {
    let Some(labels) = speaker_labels else {
        return vec![];
    };
    let count = labels
        .get("speakers")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    (0..count)
        .map(|i| Speaker {
            id: format!("spk_{i}"),
            label: format!("Speaker {}", i + 1),
        })
        .collect()
}

fn segments_by_speaker(
    items: &[serde_json::Value],
    speaker_segments: &serde_json::Value,
) -> Vec<TranscriptSegment> {
    let Some(turns) = speaker_segments.as_array() else {
        return vec![];
    };

    let mut out = Vec::new();
    let mut counter = 1usize;

    for turn in turns {
        let speaker_id = turn
            .get("speaker_label")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
        let start_seconds = parse_time_to_seconds(turn.get("start_time"));
        let end_seconds = parse_time_to_seconds(turn.get("end_time"));

        let turn_items: Vec<&serde_json::Value> = items
            .iter()
            .filter(|it| item_falls_within(it, start_seconds, end_seconds))
            .collect();

        if turn_items.is_empty() {
            continue;
        }

        let mut current_lang: Option<String> = None;
        let mut current_start: u32 = start_seconds;
        let mut current_end: u32 = start_seconds;
        let mut buffer = String::new();

        for it in turn_items {
            let lang = it
                .get("language_code")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let is_punct = it.get("type").and_then(|v| v.as_str()) == Some("punctuation");
            let content = item_content(it);

            let lang_changed = !is_punct && lang.is_some() && lang != current_lang;

            if lang_changed && !buffer.trim().is_empty() {
                out.push(TranscriptSegment {
                    id: seg_id(&mut counter),
                    speaker_id: speaker_id.clone(),
                    language_code: current_lang.clone(),
                    start_seconds: current_start,
                    end_seconds: current_end,
                    text: buffer.trim().to_string(),
                    translation: None,
                });
                buffer.clear();
                current_start = parse_time_to_seconds(it.get("start_time"));
            }

            if !is_punct {
                if current_lang.is_none() || lang_changed {
                    current_lang = lang.clone();
                }
                current_end = parse_time_to_seconds(it.get("end_time"));
                if !buffer.is_empty() {
                    buffer.push(' ');
                }
                buffer.push_str(&content);
            } else {
                buffer.push_str(&content);
            }
        }

        if !buffer.trim().is_empty() {
            out.push(TranscriptSegment {
                id: seg_id(&mut counter),
                speaker_id: speaker_id.clone(),
                language_code: current_lang.clone(),
                start_seconds: current_start,
                end_seconds,
                text: buffer.trim().to_string(),
                translation: None,
            });
        }
    }

    out
}

fn segments_by_language(items: &[serde_json::Value]) -> Vec<TranscriptSegment> {
    let mut out = Vec::new();
    let mut counter = 1usize;

    let mut current_lang: Option<String> = None;
    let mut current_start: u32 = 0;
    let mut current_end: u32 = 0;
    let mut started = false;
    let mut buffer = String::new();

    for it in items {
        let is_punct = it.get("type").and_then(|v| v.as_str()) == Some("punctuation");
        let content = item_content(it);
        let lang = it
            .get("language_code")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let lang_changed = !is_punct && lang.is_some() && lang != current_lang;

        if lang_changed && !buffer.trim().is_empty() {
            out.push(TranscriptSegment {
                id: seg_id(&mut counter),
                speaker_id: None,
                language_code: current_lang.clone(),
                start_seconds: current_start,
                end_seconds: current_end,
                text: buffer.trim().to_string(),
                translation: None,
            });
            buffer.clear();
            started = false;
        }

        if !is_punct {
            if !started {
                current_start = parse_time_to_seconds(it.get("start_time"));
                current_lang = lang.clone();
                started = true;
            }
            if lang_changed {
                current_lang = lang.clone();
            }
            current_end = parse_time_to_seconds(it.get("end_time"));
            if !buffer.is_empty() {
                buffer.push(' ');
            }
            buffer.push_str(&content);
        } else {
            buffer.push_str(&content);
        }
    }

    if !buffer.trim().is_empty() {
        out.push(TranscriptSegment {
            id: seg_id(&mut counter),
            speaker_id: None,
            language_code: current_lang,
            start_seconds: current_start,
            end_seconds: current_end,
            text: buffer.trim().to_string(),
            translation: None,
        });
    }

    out
}

fn seg_id(counter: &mut usize) -> String {
    let s = format!("seg_{:04}", *counter);
    *counter += 1;
    s
}

fn parse_time_to_seconds(value: Option<&serde_json::Value>) -> u32 {
    let s = value.and_then(|v| v.as_str()).unwrap_or("0");
    let f: f64 = s.parse().unwrap_or(0.0);
    f.floor() as u32
}

fn item_falls_within(item: &serde_json::Value, start_seconds: u32, end_seconds: u32) -> bool {
    let Some(item_start) = item.get("start_time") else {
        return false;
    };
    let s = parse_time_to_seconds(Some(item_start));
    s >= start_seconds && s < end_seconds.max(start_seconds + 1)
}

fn item_content(item: &serde_json::Value) -> String {
    item.get("alternatives")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|a| a.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Body format: render / parse the [Speaker mm:ss–mm:ss lang] header form
// ---------------------------------------------------------------------------

/// Render a [`TranscriptResult`] into the canonical plain-text-with-headers body
/// stored in the `.text` sidecar.
///
/// Format per segment:
///
/// ```text
/// [<speaker_label> <mm:ss>\u{2013}<mm:ss>[ <language_code>]]
/// <segment text>
/// ```
///
/// When there's no diarization and no language tag, headers are omitted and the
/// body is a single paragraph (legacy-compatible).
pub fn format_transcript_body(result: &TranscriptResult) -> String {
    let all_unspoken = result
        .segments
        .iter()
        .all(|s| s.speaker_id.is_none() && s.language_code.is_none());
    if all_unspoken {
        return result
            .segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
    }

    let mut out = String::new();
    let speakers: HashMap<&str, &str> = result
        .speakers
        .iter()
        .map(|s| (s.id.as_str(), s.label.as_str()))
        .collect();

    for seg in &result.segments {
        let label = seg
            .speaker_id
            .as_deref()
            .and_then(|id| speakers.get(id).copied())
            .unwrap_or("Speaker");
        let start = format_mm_ss(seg.start_seconds);
        let end = format_mm_ss(seg.end_seconds);
        match &seg.language_code {
            Some(lang) => {
                let _ = writeln!(out, "[{label} {start}\u{2013}{end} {lang}]");
            }
            None => {
                let _ = writeln!(out, "[{label} {start}\u{2013}{end}]");
            }
        }
        out.push_str(seg.text.trim());
        out.push('\n');
        if let Some(translation) = &seg.translation {
            for line in translation.trim().lines() {
                let _ = writeln!(out, "> {line}");
            }
        }
        out.push('\n');
    }

    out.trim_end().to_string()
}

fn format_mm_ss(total_seconds: u32) -> String {
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

/// Parse the canonical plain-text-with-headers body back into a
/// [`TranscriptResult`]. Tolerates:
///
/// - legacy bodies with no headers at all (one big segment)
/// - hand-edited headers with extra whitespace
/// - missing language tags
/// - speaker labels that have been renamed (e.g. "Speaker 1" → "Clinician")
///
/// Speaker IDs are reassigned by *distinct label*: the first label encountered
/// becomes `spk_0`, the second `spk_1`, and so on.
pub fn parse_transcript_body(body: &str) -> TranscriptResult {
    let lines: Vec<&str> = body.lines().collect();
    let mut segments = Vec::new();
    let mut label_to_id: Vec<(String, String)> = Vec::new();
    let mut counter = 1usize;

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.is_empty() {
            i += 1;
            continue;
        }

        if let Some((label, start_seconds, end_seconds, lang)) = parse_header(line) {
            i += 1;
            let mut text_lines: Vec<&str> = Vec::new();
            let mut translation_lines: Vec<String> = Vec::new();
            while i < lines.len() && parse_header(lines[i].trim()).is_none() {
                let raw = lines[i];
                let trimmed = raw.trim_start();
                if let Some(rest) = trimmed.strip_prefix("> ") {
                    translation_lines.push(rest.to_string());
                } else if trimmed == ">" {
                    translation_lines.push(String::new());
                } else {
                    text_lines.push(raw);
                }
                i += 1;
            }
            let text = text_lines.join("\n").trim().to_string();
            let translation = if translation_lines.is_empty() {
                None
            } else {
                Some(translation_lines.join("\n").trim().to_string())
            };

            let speaker_id = if label.is_empty() {
                None
            } else {
                Some(intern_speaker_label(&mut label_to_id, &label))
            };

            segments.push(TranscriptSegment {
                id: seg_id(&mut counter),
                speaker_id,
                language_code: lang,
                start_seconds,
                end_seconds,
                text,
                translation,
            });
        } else {
            let mut text_lines: Vec<&str> = Vec::new();
            while i < lines.len() && parse_header(lines[i].trim()).is_none() {
                text_lines.push(lines[i]);
                i += 1;
            }
            let text = text_lines.join("\n").trim().to_string();
            if !text.is_empty() {
                segments.push(TranscriptSegment {
                    id: seg_id(&mut counter),
                    speaker_id: None,
                    language_code: None,
                    start_seconds: 0,
                    end_seconds: 0,
                    text,
                    translation: None,
                });
            }
        }
    }

    let speakers = label_to_id
        .into_iter()
        .map(|(label, id)| Speaker { id, label })
        .collect();

    TranscriptResult { segments, speakers }
}

fn intern_speaker_label(table: &mut Vec<(String, String)>, label: &str) -> String {
    if let Some((_, id)) = table.iter().find(|(l, _)| l == label) {
        return id.clone();
    }
    let id = format!("spk_{}", table.len());
    table.push((label.to_string(), id.clone()));
    id
}

/// Parse a header of the form `[Label mm:ss\u{2013}mm:ss[ lang]]`.
/// Accepts en-dash, em-dash, or ASCII hyphen between the times.
fn parse_header(line: &str) -> Option<(String, u32, u32, Option<String>)> {
    let line = line.trim();
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;

    let (label_part, time_part) = split_label_and_time(inner)?;
    let (start_str, rest) = split_first_mm_ss(time_part)?;
    let rest = rest.trim_start_matches(['\u{2013}', '\u{2014}', '-']);
    let (end_str, tail) = split_first_mm_ss(rest)?;
    let lang = tail.trim();
    let lang_opt = if lang.is_empty() {
        None
    } else {
        Some(lang.to_string())
    };

    Some((
        label_part.trim().to_string(),
        mm_ss_to_seconds(start_str)?,
        mm_ss_to_seconds(end_str)?,
        lang_opt,
    ))
}

fn split_label_and_time(inner: &str) -> Option<(&str, &str)> {
    for (i, _) in inner.char_indices() {
        let candidate = &inner[i..];
        if looks_like_mm_ss(candidate) {
            let label = &inner[..i];
            let trimmed_end = label.trim_end();
            let end = trimmed_end.len();
            return Some((&inner[..end], &inner[i..]));
        }
    }
    None
}

fn looks_like_mm_ss(s: &str) -> bool {
    let (digits, rest) = split_leading_digits(s);
    if !(1..=3).contains(&digits.len()) {
        return false;
    }
    if !rest.starts_with(':') {
        return false;
    }
    let after = &rest[1..];
    let (sec, _) = split_leading_digits(after);
    sec.len() == 2
}

fn split_leading_digits(s: &str) -> (&str, &str) {
    let end = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

fn split_first_mm_ss(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if !looks_like_mm_ss(s) {
        return None;
    }
    let (mins, rest1) = split_leading_digits(s);
    let rest1 = &rest1[1..];
    let (secs, _rest2) = split_leading_digits(rest1);
    let combined_len = mins.len() + 1 + secs.len();
    Some((&s[..combined_len], &s[combined_len..]))
}

fn mm_ss_to_seconds(s: &str) -> Option<u32> {
    let (mins, rest) = s.split_once(':')?;
    let mins: u32 = mins.parse().ok()?;
    let secs: u32 = rest.parse().ok()?;
    Some(mins * 60 + secs)
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
