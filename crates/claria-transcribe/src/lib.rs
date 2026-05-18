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
    LanguageCode, LanguageIdSettings, Media, MedicalContentIdentificationType,
    MedicalTranscriptionSetting, Settings, Specialty, TranscriptionJobStatus,
    Type as MedicalType,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write as _;
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
    #[serde(default)]
    pub custom_vocabulary: Option<String>,
}

impl Default for TranscribeOptions {
    fn default() -> Self {
        Self {
            language: LanguageMode::English,
            speakers: SpeakerHandling::Diarize { max: 2 },
            engine: TranscriptionEngine::Standard,
            custom_vocabulary: None,
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
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
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
        .map_err(|e| TranscribeError::Api(format!("failed to read transcript from S3: {e}")))?;

    let body_str = String::from_utf8(transcript_json.body)
        .map_err(|e| TranscribeError::Parse(e.to_string()))?;

    let result = parse_transcribe_json(&body_str)?;

    // Clean up temporary job + intermediate JSON.
    let _ = claria_storage::objects::delete_object(&s3, bucket, &output_key).await;
    match engine {
        TranscriptionEngine::Standard => {
            let _ = transcribe
                .delete_transcription_job()
                .transcription_job_name(&job_name)
                .send()
                .await;
        }
        TranscriptionEngine::Medical => {
            let _ = transcribe
                .delete_medical_transcription_job()
                .medical_transcription_job_name(&job_name)
                .send()
                .await;
        }
    }

    Ok(result)
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
        LanguageMode::Mixed => {
            let mut r = req
                .identify_multiple_languages(true)
                .language_options(LanguageCode::EnUs)
                .language_options(LanguageCode::EsUs);
            if let Some(vocab) = options.custom_vocabulary.as_deref() {
                let settings = LanguageIdSettings::builder()
                    .vocabulary_name(vocab)
                    .build();
                let mut m = HashMap::new();
                m.insert(LanguageCode::EnUs, settings.clone());
                m.insert(LanguageCode::EsUs, settings);
                r = r.set_language_id_settings(Some(m));
            }
            r
        }
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

    if !matches!(options.language, LanguageMode::Mixed)
        && let Some(vocab) = options.custom_vocabulary.as_deref()
    {
        builder = builder.vocabulary_name(vocab);
        touched = true;
    }

    if touched { Some(builder.build()) } else { None }
}

async fn poll_standard_job(
    transcribe: &aws_sdk_transcribe::Client,
    job_name: &str,
) -> Result<(), TranscribeError> {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let resp = transcribe
            .get_transcription_job()
            .transcription_job_name(job_name)
            .send()
            .await
            .map_err(|e| TranscribeError::Api(e.into_service_error().to_string()))?;

        let job = resp
            .transcription_job()
            .ok_or_else(|| TranscribeError::Api("no job in response".into()))?;

        match job.transcription_job_status() {
            Some(TranscriptionJobStatus::Completed) => return Ok(()),
            Some(TranscriptionJobStatus::Failed) => {
                let reason = job.failure_reason().unwrap_or("unknown").to_string();
                let _ = transcribe
                    .delete_transcription_job()
                    .transcription_job_name(job_name)
                    .send()
                    .await;
                return Err(TranscribeError::JobFailed(reason));
            }
            _ => continue,
        }
    }
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

    if let Some(vocab) = options.custom_vocabulary.as_deref() {
        builder = builder.vocabulary_name(vocab);
        touched = true;
    }

    if touched { Some(builder.build()) } else { None }
}

async fn poll_medical_job(
    transcribe: &aws_sdk_transcribe::Client,
    job_name: &str,
) -> Result<(), TranscribeError> {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let resp = transcribe
            .get_medical_transcription_job()
            .medical_transcription_job_name(job_name)
            .send()
            .await
            .map_err(|e| TranscribeError::Api(e.into_service_error().to_string()))?;

        let job = resp
            .medical_transcription_job()
            .ok_or_else(|| TranscribeError::Api("no medical job in response".into()))?;

        match job.transcription_job_status() {
            Some(TranscriptionJobStatus::Completed) => return Ok(()),
            Some(TranscriptionJobStatus::Failed) => {
                let reason = job.failure_reason().unwrap_or("unknown").to_string();
                let _ = transcribe
                    .delete_medical_transcription_job()
                    .medical_transcription_job_name(job_name)
                    .send()
                    .await;
                return Err(TranscribeError::JobFailed(reason));
            }
            _ => continue,
        }
    }
}

// ---------------------------------------------------------------------------
// Transcribe JSON → structured TranscriptResult
// ---------------------------------------------------------------------------

/// Parse Amazon Transcribe's output JSON (works for both standard and medical
/// responses) into a structured [`TranscriptResult`].
fn parse_transcribe_json(json: &str) -> Result<TranscriptResult, TranscribeError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| TranscribeError::Parse(e.to_string()))?;

    let results = value
        .get("results")
        .ok_or_else(|| TranscribeError::Parse("missing `results` block".into()))?;

    let items = results.get("items").and_then(|i| i.as_array());
    let speaker_labels = results.get("speaker_labels");

    let speakers = extract_speakers(speaker_labels);

    let segments = if let (Some(items), Some(labels)) =
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
            start_ms: 0,
            end_ms: 0,
            text,
        }]
    };

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
        let start_ms = parse_time_to_ms(turn.get("start_time"));
        let end_ms = parse_time_to_ms(turn.get("end_time"));

        let turn_items: Vec<&serde_json::Value> = items
            .iter()
            .filter(|it| item_falls_within(it, start_ms, end_ms))
            .collect();

        if turn_items.is_empty() {
            continue;
        }

        let mut current_lang: Option<String> = None;
        let mut current_start: u64 = start_ms;
        let mut current_end: u64 = start_ms;
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
                    start_ms: current_start,
                    end_ms: current_end,
                    text: buffer.trim().to_string(),
                });
                buffer.clear();
                current_start = parse_time_to_ms(it.get("start_time"));
            }

            if !is_punct {
                if current_lang.is_none() || lang_changed {
                    current_lang = lang.clone();
                }
                current_end = parse_time_to_ms(it.get("end_time"));
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
                start_ms: current_start,
                end_ms,
                text: buffer.trim().to_string(),
            });
        }
    }

    out
}

fn segments_by_language(items: &[serde_json::Value]) -> Vec<TranscriptSegment> {
    let mut out = Vec::new();
    let mut counter = 1usize;

    let mut current_lang: Option<String> = None;
    let mut current_start: u64 = 0;
    let mut current_end: u64 = 0;
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
                start_ms: current_start,
                end_ms: current_end,
                text: buffer.trim().to_string(),
            });
            buffer.clear();
            started = false;
        }

        if !is_punct {
            if !started {
                current_start = parse_time_to_ms(it.get("start_time"));
                current_lang = lang.clone();
                started = true;
            }
            if lang_changed {
                current_lang = lang.clone();
            }
            current_end = parse_time_to_ms(it.get("end_time"));
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
            start_ms: current_start,
            end_ms: current_end,
            text: buffer.trim().to_string(),
        });
    }

    out
}

fn seg_id(counter: &mut usize) -> String {
    let s = format!("seg_{:04}", *counter);
    *counter += 1;
    s
}

fn parse_time_to_ms(value: Option<&serde_json::Value>) -> u64 {
    let s = value.and_then(|v| v.as_str()).unwrap_or("0");
    let f: f64 = s.parse().unwrap_or(0.0);
    (f * 1000.0).round() as u64
}

fn item_falls_within(item: &serde_json::Value, start_ms: u64, end_ms: u64) -> bool {
    let Some(item_start) = item.get("start_time") else {
        return false;
    };
    let s = parse_time_to_ms(Some(item_start));
    s >= start_ms && s < end_ms.max(start_ms + 1)
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
        let start = format_mm_ss(seg.start_ms);
        let end = format_mm_ss(seg.end_ms);
        match &seg.language_code {
            Some(lang) => {
                let _ = writeln!(out, "[{label} {start}\u{2013}{end} {lang}]");
            }
            None => {
                let _ = writeln!(out, "[{label} {start}\u{2013}{end}]");
            }
        }
        out.push_str(seg.text.trim());
        out.push_str("\n\n");
    }

    out.trim_end().to_string()
}

fn format_mm_ss(ms: u64) -> String {
    let total_seconds = ms / 1000;
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

        if let Some((label, start_ms, end_ms, lang)) = parse_header(line) {
            i += 1;
            let mut text_lines: Vec<&str> = Vec::new();
            while i < lines.len() && parse_header(lines[i].trim()).is_none() {
                text_lines.push(lines[i]);
                i += 1;
            }
            let text = text_lines.join("\n").trim().to_string();

            let speaker_id = if label.is_empty() {
                None
            } else {
                Some(intern_speaker_label(&mut label_to_id, &label))
            };

            segments.push(TranscriptSegment {
                id: seg_id(&mut counter),
                speaker_id,
                language_code: lang,
                start_ms,
                end_ms,
                text,
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
                    start_ms: 0,
                    end_ms: 0,
                    text,
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
fn parse_header(line: &str) -> Option<(String, u64, u64, Option<String>)> {
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
        mm_ss_to_ms(start_str)?,
        mm_ss_to_ms(end_str)?,
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

fn mm_ss_to_ms(s: &str) -> Option<u64> {
    let (mins, rest) = s.split_once(':')?;
    let mins: u64 = mins.parse().ok()?;
    let secs: u64 = rest.parse().ok()?;
    Some((mins * 60 + secs) * 1000)
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
