//! Transcription option mapping, translation, and the audio file picker.

use serde::Deserialize;

use claria_desktop::config::{self, TranscriptionPreferences};

use super::{CommandContext, usage_audit_details};

/// The Bedrock model ID used for per-segment transcript translation.
///
/// Pinned to Claude Sonnet 4.6: handles specialized vocabulary (drug names,
/// anatomy, dosage phrases) more reliably than Haiku, at a cost rounding-error
/// compared to the Transcribe spend per session. Same rationale as
/// [`super::records::EXTRACTION_MODEL_ID`] — internal operations get pinned to
/// a sensible model rather than exposing yet another preference knob.
const TRANSLATION_MODEL_ID: &str = "us.anthropic.claude-sonnet-4-6";

/// Per-file overrides for the wizard flow. Each field is optional so the
/// frontend only sends what the user actually changed; everything else falls
/// back to the saved preferences. Uses the `TranscriptionLanguage` type from
/// our config crate (specta-typed) rather than the library's `LanguageMode` —
/// the wrapper keeps the TS binding inside the desktop crate's surface.
#[derive(Debug, Clone, Deserialize, specta::Type)]
pub struct TranscribeOptionsOverrides {
    #[serde(default)]
    pub language: Option<config::TranscriptionLanguage>,
    #[serde(default)]
    pub speaker_mode: Option<SpeakerMode>,
    #[serde(default)]
    pub speaker_count: Option<u8>,
    #[serde(default)]
    pub use_medical_for_english: Option<bool>,
    /// When set, overrides `prefs.translate_to_english` for this single file.
    #[serde(default)]
    pub translate_to_english: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerMode {
    None,
    Diarize,
    Channels,
}

/// Map per-clinician `TranscriptionPreferences` + per-file overrides into the
/// `TranscribeOptions` shape the library crate expects.
pub(crate) fn build_transcribe_options(
    prefs: &TranscriptionPreferences,
    overrides: Option<TranscribeOptionsOverrides>,
) -> claria_transcribe::TranscribeOptions {
    let lang_pref = overrides
        .as_ref()
        .and_then(|o| o.language)
        .unwrap_or(prefs.default_language);
    let language = match lang_pref {
        config::TranscriptionLanguage::English => claria_transcribe::LanguageMode::English,
        config::TranscriptionLanguage::Spanish => claria_transcribe::LanguageMode::Spanish,
        config::TranscriptionLanguage::Mixed => claria_transcribe::LanguageMode::Mixed,
    };

    let speaker_count = overrides
        .as_ref()
        .and_then(|o| o.speaker_count)
        .unwrap_or(prefs.default_speaker_count);

    let speakers = match overrides.as_ref().and_then(|o| o.speaker_mode) {
        Some(SpeakerMode::None) => claria_transcribe::SpeakerHandling::None,
        Some(SpeakerMode::Channels) => claria_transcribe::SpeakerHandling::Channels,
        Some(SpeakerMode::Diarize) | None => match speaker_count {
            0 | 1 => claria_transcribe::SpeakerHandling::None,
            n => claria_transcribe::SpeakerHandling::Diarize { max: n },
        },
    };

    let use_medical = overrides
        .as_ref()
        .and_then(|o| o.use_medical_for_english)
        .unwrap_or(prefs.use_medical_for_english);
    let engine = if use_medical {
        claria_transcribe::TranscriptionEngine::Medical
    } else {
        claria_transcribe::TranscriptionEngine::Standard
    };

    claria_transcribe::TranscribeOptions {
        language,
        speakers,
        engine,
    }
}

/// Translate non-English segments in-place if translation is enabled.
pub(crate) async fn maybe_translate(
    ctx: &CommandContext,
    result: &mut claria_transcribe::TranscriptResult,
    translate: bool,
) {
    if !translate {
        return;
    }
    let model_id = TRANSLATION_MODEL_ID;

    let requests: Vec<claria_bedrock::translate::TranslationRequest> = result
        .segments
        .iter()
        .enumerate()
        .filter_map(|(idx, seg)| {
            let lang = seg.language_code.as_deref()?;
            if lang == "en-US" || lang.starts_with("en-") || seg.text.trim().is_empty() {
                return None;
            }
            Some(claria_bedrock::translate::TranslationRequest {
                index: idx,
                language_code: lang.to_string(),
                source_text: seg.text.clone(),
            })
        })
        .collect();

    if requests.is_empty() {
        return;
    }

    match claria_bedrock::translate::translate_segments(&ctx.sdk_config, model_id, &requests).await
    {
        Ok((outputs, usage)) => {
            for output in &outputs {
                if let Some(seg) = result.segments.get_mut(output.index) {
                    seg.translation = Some(output.translation.clone());
                }
            }
            let mut audit_details = usage_audit_details(model_id, usage.as_ref());
            audit_details["segment_count"] = serde_json::json!(outputs.len());
            ctx.record_audit(
                ctx.audit_event("translate_transcript", "transcript", "")
                    .with_details(audit_details),
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(error = %e, "translation failed; sidecar will be written without translations");
        }
    }
}

/// Open a native file picker scoped to supported audio formats. Returns the
/// absolute path the user chose, or `None` if they cancelled.
///
/// Used by the transcription wizard so we can keep a real file picker on the
/// wizard surface (avoiding the geometry-sensitive drag-target controls flagged
/// in [feedback-ui-low-dexterity]).
#[tauri::command]
#[specta::specta]
pub fn pick_audio_file() -> Result<Option<String>, String> {
    let path = rfd::FileDialog::new()
        .set_title("Choose an audio file to transcribe")
        .add_filter(
            "Audio",
            &["mp3", "m4a", "mp4", "wav", "flac", "ogg", "amr", "webm"],
        )
        .pick_file();
    Ok(path.and_then(|p| p.to_str().map(|s| s.to_string())))
}
