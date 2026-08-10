//! Per-segment translation of transcribed audio via Bedrock Converse.
//!
//! Used when a clinician has `translate_to_english` enabled and a transcription
//! contains non-English segments. We send all foreign segments to Bedrock in a
//! single Converse call and force a `submit_translations` tool call via
//! `tool_choice`, so the structured envelope comes back as typed tool input —
//! never parsed out of free text.
//!
//! The caller owns the [`TranscriptSegment`] type; this module takes opaque
//! `(index, language_code, source_text)` tuples and returns the translations
//! by index so we never depend on `claria-transcribe`.

use std::collections::BTreeSet;

use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, InferenceConfiguration, Message, SpecificToolChoice,
    StopReason, SystemContentBlock, Tool, ToolChoice, ToolConfiguration, ToolInputSchema,
    ToolSpecification,
};
use claria_core::models::turn_usage::TurnUsage;
use serde::Deserialize;
use tracing::info;

use crate::{converse, error::BedrockError};

/// Name of the forced tool that carries the translation envelope.
pub const SUBMIT_TRANSLATIONS_TOOL: &str = "submit_translations";

/// Output-token ceiling for one translation Converse call. Batches are
/// per-segment utterances, so translations fit comfortably below this.
pub const TRANSLATE_MAX_OUTPUT_TOKENS: u32 = 8_192;

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
Submit your translations by calling the submit_translations tool exactly once, \
with one entry per input segment whose index is copied from that segment. \
Preserve names, dosages, and numbers verbatim. Do not add commentary, do not \
summarise, do not interpret.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    translations: Vec<EnvelopeItem>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeItem {
    index: usize,
    translation: String,
}

/// Translate a batch of segments in a single Bedrock Converse call.
///
/// The response is a forced `submit_translations` tool call; after decoding,
/// the returned index set must equal the requested set — duplicates, unknown
/// indexes, and gaps are all hard errors so no segment silently loses its
/// translation. An empty `requests` slice short-circuits to an empty result.
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
        "Translate the following segments into English.\n\n{}",
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
        .tool_config(translation_tool_configuration()?)
        .inference_config(
            InferenceConfiguration::builder()
                .max_tokens(TRANSLATE_MAX_OUTPUT_TOKENS as i32)
                .build(),
        )
        .send()
        .await
        .map_err(|error| converse::classify_error("translation Converse", error))?;

    match response.stop_reason() {
        StopReason::ToolUse => {}
        StopReason::MaxTokens => {
            return Err(BedrockError::ResponseTruncated {
                max_output_tokens: TRANSLATE_MAX_OUTPUT_TOKENS,
            });
        }
        StopReason::ModelContextWindowExceeded => {
            return Err(BedrockError::ContextWindowExceeded);
        }
        other => {
            return Err(BedrockError::ResponseParse(format!(
                "unexpected stop reason {other:?} for a forced translation tool call"
            )));
        }
    }

    let output_message = response
        .output()
        .and_then(|o| o.as_message().ok())
        .ok_or_else(|| BedrockError::ResponseParse("no message in response".into()))?;
    let tool_input = output_message
        .content()
        .iter()
        .find_map(|block| match block {
            ContentBlock::ToolUse(tool) if tool.name() == SUBMIT_TRANSLATIONS_TOOL => {
                Some(tool.input())
            }
            _ => None,
        })
        .ok_or_else(|| {
            BedrockError::ResponseParse(
                "forced translation response carried no submit_translations tool call".into(),
            )
        })?;

    let envelope: Envelope = serde_json::from_value(converse::document_to_json(tool_input)?)
        .map_err(|error| {
            BedrockError::SchemaViolation(format!(
                "translation envelope did not match schema: {error}"
            ))
        })?;

    // The returned index set must equal the requested set. A dropped index
    // would silently leave a foreign-language segment untranslated in the
    // persisted transcript.
    let requested: BTreeSet<usize> = requests.iter().map(|request| request.index).collect();
    let mut returned = BTreeSet::new();
    for item in &envelope.translations {
        if !requested.contains(&item.index) {
            return Err(BedrockError::SchemaViolation(format!(
                "translation returned for unrequested segment index {}",
                item.index
            )));
        }
        if !returned.insert(item.index) {
            return Err(BedrockError::SchemaViolation(format!(
                "duplicate translation for segment index {}",
                item.index
            )));
        }
    }
    let missing: Vec<usize> = requested.difference(&returned).copied().collect();
    if !missing.is_empty() {
        return Err(BedrockError::SchemaViolation(format!(
            "translations missing for segment indexes {missing:?}"
        )));
    }

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

/// The single-tool configuration with `tool_choice` forcing
/// `submit_translations`, so structure never has to be parsed out of free
/// text.
fn translation_tool_configuration() -> Result<ToolConfiguration, BedrockError> {
    let schema = serde_json::json!({
        "type": "object",
        "required": ["translations"],
        "additionalProperties": false,
        "properties": {
            "translations": {
                "type": "array",
                "minItems": 1,
                "description": "Exactly one entry per input segment. Every requested index must appear exactly once.",
                "items": {
                    "type": "object",
                    "required": ["index", "translation"],
                    "additionalProperties": false,
                    "properties": {
                        "index": {
                            "type": "integer", "minimum": 0,
                            "description": "The index copied exactly from the input segment being translated."
                        },
                        "translation": {
                            "type": "string",
                            "description": "The English translation of that segment's source_text, with names, dosages, and numbers preserved verbatim."
                        }
                    }
                }
            }
        }
    });
    let specification = ToolSpecification::builder()
        .name(SUBMIT_TRANSLATIONS_TOOL)
        .description(
            "Submit the English translations for all input segments. Called exactly once per \
             request, with one entry for every requested segment index.",
        )
        .input_schema(ToolInputSchema::Json(converse::json_to_document(&schema)?))
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))?;
    let tool_choice = ToolChoice::Tool(
        SpecificToolChoice::builder()
            .name(SUBMIT_TRANSLATIONS_TOOL)
            .build()
            .map_err(|error| BedrockError::Invocation(error.to_string()))?,
    );
    ToolConfiguration::builder()
        .tools(Tool::ToolSpec(specification))
        .tool_choice(tool_choice)
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))
}
