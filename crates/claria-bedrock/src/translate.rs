//! Per-segment translation of transcribed audio via Bedrock Converse.
//!
//! Used when a clinician has `translate_to_english` enabled and a transcription
//! contains non-English segments. We send all foreign segments to Bedrock in a
//! single Converse call (one prompt, one response, indexed JSON output) to
//! amortize the per-request overhead.
//!
//! The caller owns the [`TranscriptSegment`] type; this module takes opaque
//! `(index, language_code, source_text)` tuples and returns the translations
//! by index so we never depend on `claria-transcribe`.

use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, InferenceConfiguration, Message, SystemContentBlock,
};
use claria_core::models::turn_usage::TurnUsage;
use serde::Deserialize;
use tracing::info;

use crate::{converse, error::BedrockError};

/// A segment-to-translate, identified by its position in the caller's segment
/// list. Index numbers travel through Bedrock as-is so the caller can stitch
/// translations back onto the original segments without re-parsing language
/// codes.
#[derive(Debug, Clone)]
pub struct TranslationRequest {
    pub index: usize,
    pub language_code: String,
    pub source_text: String,
}

/// One translation result. Always 1:1 with a request.
#[derive(Debug, Clone)]
pub struct TranslationOutput {
    pub index: usize,
    pub translation: String,
}

const SYSTEM_PROMPT: &str = "\
You translate short clinical-conversation utterances into English. \
You return JSON only: {\"translations\":[{\"index\":N,\"translation\":\"...\"}, ...]} \
where each index corresponds to the input index. Preserve names, dosages, and \
numbers verbatim. Do not add commentary, do not summarise, do not interpret.";

#[derive(Debug, Deserialize)]
struct Envelope {
    translations: Vec<EnvelopeItem>,
}

#[derive(Debug, Deserialize)]
struct EnvelopeItem {
    index: usize,
    translation: String,
}

/// Translate a batch of segments in a single Bedrock Converse call.
///
/// Returns the translations and the token-usage block for billing/audit. An
/// empty `requests` slice short-circuits to an empty result with zeroed usage.
pub async fn translate_segments(
    config: &aws_config::SdkConfig,
    model_id: &str,
    requests: &[TranslationRequest],
) -> Result<(Vec<TranslationOutput>, Option<TurnUsage>), BedrockError> {
    if requests.is_empty() {
        return Ok((Vec::new(), None));
    }

    let client = converse::runtime_client(config);

    let user_payload = serde_json::json!({
        "segments": requests.iter().map(|r| serde_json::json!({
            "index": r.index,
            "language_code": r.language_code,
            "source_text": r.source_text,
        })).collect::<Vec<_>>(),
    });

    let user_text = format!(
        "Translate the following segments into English. Respond with JSON only.\n\n{}",
        serde_json::to_string(&user_payload)?
    );

    let message = Message::builder()
        .role(ConversationRole::User)
        .content(ContentBlock::Text(user_text))
        .build()
        .map_err(|e| BedrockError::Invocation(e.to_string()))?;

    info!(model_id, count = requests.len(), "translating segments");

    let response = client
        .converse()
        .model_id(model_id)
        .system(SystemContentBlock::Text(SYSTEM_PROMPT.to_string()))
        .messages(message)
        .inference_config(
            InferenceConfiguration::builder()
                .temperature(0.0)
                .build(),
        )
        .send()
        .await
        .map_err(|error| converse::classify_error("translation Converse", error))?;

    let output_message = response
        .output()
        .and_then(|o| o.as_message().ok())
        .ok_or_else(|| BedrockError::ResponseParse("no message in response".into()))?;
    let output_text = converse::collect_text(output_message);

    let envelope: Envelope = parse_translation_envelope(&output_text)?;

    let outputs: Vec<TranslationOutput> = envelope
        .translations
        .into_iter()
        .map(|t| TranslationOutput {
            index: t.index,
            translation: t.translation,
        })
        .collect();

    let usage = converse::optional_usage(response.usage(), model_id);

    Ok((outputs, usage))
}

/// Tolerate fenced code blocks (```json ... ```) around the JSON envelope.
fn parse_translation_envelope(raw: &str) -> Result<Envelope, BedrockError> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim_start_matches('\n'))
        .and_then(|s| s.strip_suffix("```").or(Some(s)))
        .unwrap_or(trimmed);

    serde_json::from_str(stripped.trim()).map_err(|e| {
        BedrockError::SchemaViolation(format!(
            "translation envelope did not match schema: {e}"
        ))
    })
}
