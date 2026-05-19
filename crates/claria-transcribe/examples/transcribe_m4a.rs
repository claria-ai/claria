//! Fire a local audio file at real AWS Transcribe in Mixed mode (en-US + es-US,
//! 2-speaker diarization) and dump the raw transcript JSON so we can see what
//! AWS actually returned. Same request shape as the production
//! `transcribe_audio_with_options(..., LanguageMode::Mixed,
//! SpeakerHandling::Diarize { max: 2 }, TranscriptionEngine::Standard)` path.
//!
//! Usage:
//!
//!     cargo run --example transcribe_m4a -- <bucket> [<audio.m4a>] [--keep]
//!
//! If `<audio.m4a>` is omitted, the bundled
//! `tests/assets/133 Oak Hill Ave 2.m4a` fixture is used.
//!
//! Uses the standard AWS credential chain (env / shared config / SSO). Uploads
//! the audio under `_debug-transcribe/<uuid>/<filename>` in `<bucket>`, runs the
//! job, downloads the transcript JSON to
//! `./<filename>.transcribe.json`, prints AWS's reported language-ID scores +
//! per-item language histogram + a parsed segment summary, and (by default)
//! deletes the S3 artifacts and the Transcribe job record. Pass `--keep` to
//! leave them in S3.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use aws_config::BehaviorVersion;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_transcribe::types::{
    LanguageCode, Media, MediaFormat, Settings, TranscriptionJobStatus,
};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut positional: Vec<String> = Vec::new();
    let mut keep = false;
    for arg in std::env::args().skip(1) {
        if arg == "--keep" {
            keep = true;
        } else {
            positional.push(arg);
        }
    }

    if positional.is_empty() {
        eprintln!("Usage: transcribe_m4a <bucket> [<audio.m4a>] [--keep]");
        std::process::exit(2);
    }
    let bucket = positional[0].clone();
    let audio_path: PathBuf = if positional.len() >= 2 {
        PathBuf::from(&positional[1])
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("assets")
            .join("133 Oak Hill Ave 2.m4a")
    };

    let filename = audio_path
        .file_name()
        .ok_or("audio path has no filename")?
        .to_string_lossy()
        .into_owned();
    let job_id = Uuid::new_v4();
    let audio_key = format!("_debug-transcribe/{job_id}/{filename}");
    let output_key = format!("_debug-transcribe/{job_id}/transcript.json");
    let job_name = format!("claria-debug-bilingual-{job_id}");

    let config = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let s3 = aws_sdk_s3::Client::new(&config);
    let transcribe = aws_sdk_transcribe::Client::new(&config);

    eprintln!("→ uploading {} to s3://{bucket}/{audio_key}", audio_path.display());
    let bytes = std::fs::read(&audio_path)?;
    s3.put_object()
        .bucket(&bucket)
        .key(&audio_key)
        .body(ByteStream::from(bytes))
        .send()
        .await?;

    let s3_uri = format!("s3://{bucket}/{audio_key}");
    eprintln!("→ starting transcription job {job_name}");
    eprintln!("    IdentifyMultipleLanguages=true, LanguageOptions=[en-US, es-US]");
    eprintln!("    Settings.ShowSpeakerLabels=true, MaxSpeakerLabels=2");
    transcribe
        .start_transcription_job()
        .transcription_job_name(&job_name)
        .media(Media::builder().media_file_uri(&s3_uri).build())
        .media_format(MediaFormat::Mp4)
        .identify_multiple_languages(true)
        .language_options(LanguageCode::EnUs)
        .language_options(LanguageCode::EsUs)
        .output_bucket_name(&bucket)
        .output_key(&output_key)
        .settings(
            Settings::builder()
                .show_speaker_labels(true)
                .max_speaker_labels(2)
                .build(),
        )
        .send()
        .await?;

    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let resp = transcribe
            .get_transcription_job()
            .transcription_job_name(&job_name)
            .send()
            .await?;
        let job = resp.transcription_job().ok_or("no job in response")?;
        match job.transcription_job_status() {
            Some(TranscriptionJobStatus::Completed) => {
                eprintln!("→ job completed");
                if let Some(lang) = job.language_code() {
                    eprintln!("    AWS reported LanguageCode = {lang:?}");
                }
                if let Some(scores) = job.language_id_settings() {
                    eprintln!("    LanguageIdSettings: {scores:?}");
                }
                break;
            }
            Some(TranscriptionJobStatus::Failed) => {
                let reason = job.failure_reason().unwrap_or("unknown");
                return Err(format!("transcription failed: {reason}").into());
            }
            other => eprintln!("    status: {other:?} ... polling"),
        }
    }

    eprintln!("→ downloading transcript json from s3://{bucket}/{output_key}");
    let resp = s3
        .get_object()
        .bucket(&bucket)
        .key(&output_key)
        .send()
        .await?;
    let body = resp.body.collect().await?.into_bytes();
    let local_path = format!("{filename}.transcribe.json");
    std::fs::write(&local_path, &body)?;
    eprintln!("→ wrote {local_path} ({} bytes)", body.len());

    let json_str = String::from_utf8(body.to_vec())?;
    let raw: serde_json::Value = serde_json::from_str(&json_str)?;

    // Language-ID block (top-level in real AWS output)
    if let Some(ids) = raw
        .get("results")
        .and_then(|r| r.get("language_identification"))
        .and_then(|v| v.as_array())
    {
        println!("\n=== AWS language_identification ===");
        for entry in ids {
            println!("  {entry}");
        }
    } else {
        println!("\n=== AWS language_identification: (absent) ===");
    }

    // Per-item language histogram — tells us whether AWS tagged any items as es-US
    let mut hist: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(items) = raw
        .get("results")
        .and_then(|r| r.get("items"))
        .and_then(|v| v.as_array())
    {
        for it in items {
            let lang = it
                .get("language_code")
                .and_then(|v| v.as_str())
                .unwrap_or("(none)");
            *hist.entry(lang.to_string()).or_default() += 1;
        }
    }
    println!("\n=== Per-item language_code histogram ===");
    if hist.is_empty() {
        println!("  (no items in transcript)");
    } else {
        for (lang, count) in &hist {
            println!("  {lang}: {count}");
        }
    }

    // Parsed segments via the production parser
    let result = claria_transcribe::parse_transcribe_json(&json_str)
        .map_err(|e| format!("parse: {e}"))?;
    println!("\n=== Parsed segments ({} speakers, {} segments) ===",
        result.speakers.len(),
        result.segments.len()
    );
    for s in &result.speakers {
        println!("  speaker {}: {}", s.id, s.label);
    }
    for seg in &result.segments {
        let lang = seg.language_code.as_deref().unwrap_or("?");
        let spk = seg.speaker_id.as_deref().unwrap_or("?");
        println!(
            "  [{spk} {:02}:{:02}-{:02}:{:02} {lang}] {}",
            seg.start_seconds / 60,
            seg.start_seconds % 60,
            seg.end_seconds / 60,
            seg.end_seconds % 60,
            seg.text
        );
    }

    if !keep {
        eprintln!("\n→ cleaning up S3 artifacts (pass --keep to retain)");
        let _ = s3.delete_object().bucket(&bucket).key(&audio_key).send().await;
        let _ = s3.delete_object().bucket(&bucket).key(&output_key).send().await;
        let _ = transcribe
            .delete_transcription_job()
            .transcription_job_name(&job_name)
            .send()
            .await;
    } else {
        eprintln!("\n→ --keep: leaving s3://{bucket}/{audio_key} and s3://{bucket}/{output_key}");
    }

    Ok(())
}
