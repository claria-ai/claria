//! Shared Bedrock Converse plumbing for every claria-bedrock flow.
//!
//! One owner for runtime-client construction, structured service-error
//! classification, response-text collection, optional usage extraction, and
//! the `CountTokens` wrapper. Chat, extraction, translation, and the report
//! writer all speak to Bedrock through this module so their error and usage
//! semantics cannot drift apart.

use std::collections::HashMap;

use aws_sdk_bedrockruntime::{
    error::{DisplayErrorContext, ProvideErrorMetadata, SdkError},
    operation::RequestId,
    types::{
        CachePointBlock, CachePointType, CacheTtl, ContentBlock, ContentBlockDelta,
        ConverseStreamOutput, ConverseTokensRequest, CountTokensInput, Message, StopReason,
        TokenUsage,
    },
};
use aws_smithy_types::{Document, Number};
use claria_core::{model_id::CacheTtlChoice, models::turn_usage::TurnUsage};

use crate::{error::BedrockError, tokens};

/// Build the Bedrock Runtime client every Converse flow uses.
pub(crate) fn runtime_client(config: &aws_config::SdkConfig) -> aws_sdk_bedrockruntime::Client {
    aws_sdk_bedrockruntime::Client::new(config)
}

/// Classify any Bedrock SDK failure into the one structured error shape.
///
/// Service errors preserve HTTP status, service code, request ID, and the
/// service message. Non-service failures (connect/timeout/deserialization)
/// keep their full [`DisplayErrorContext`] chain instead of collapsing to
/// "unhandled error".
pub(crate) fn classify_error<E>(operation: &'static str, error: SdkError<E>) -> BedrockError
where
    E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
{
    let status = error
        .raw_response()
        .map(|response| response.status().as_u16());
    let (code, request_id, message) = match error.as_service_error() {
        Some(service_error) => (
            service_error
                .meta()
                .code()
                .unwrap_or("UnknownServiceError")
                .to_string(),
            service_error.meta().request_id().map(ToString::to_string),
            service_error
                .meta()
                .message()
                .unwrap_or("the service returned no error message")
                .to_string(),
        ),
        None => (
            "DispatchFailure".to_string(),
            None,
            DisplayErrorContext(&error).to_string(),
        ),
    };
    tracing::error!(operation, ?status, code, ?request_id, "Bedrock call failed");
    BedrockError::Service {
        operation,
        status,
        code,
        request_id,
        message,
    }
}

/// Map a domain TTL choice to the wire TTL for a cache point.
///
/// `FiveMinutes` maps to `None` — the server default — so requests that use
/// the default TTL stay byte-identical to the pre-TTL wire shape.
pub(crate) fn sdk_cache_ttl(choice: CacheTtlChoice) -> Option<CacheTtl> {
    match choice {
        CacheTtlChoice::FiveMinutes => None,
        CacheTtlChoice::OneHour => Some(CacheTtl::OneHour),
    }
}

/// Build one prompt-cache `cachePoint` block, with an optional extended
/// TTL. `None` omits the ttl field entirely (server default: 5 minutes).
pub(crate) fn cache_point(ttl: Option<CacheTtl>) -> Result<CachePointBlock, BedrockError> {
    let mut builder = CachePointBlock::builder().r#type(CachePointType::Default);
    if let Some(ttl) = ttl {
        builder = builder.ttl(ttl);
    }
    builder
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))
}

/// Append a cache point to the final message's content so the next request
/// in the conversation reads everything up to (and including) this message
/// from cache. Shared by the chat and report flows; an empty conversation
/// is returned unchanged.
pub(crate) fn with_tail_cache_point(
    mut messages: Vec<Message>,
    ttl: Option<CacheTtl>,
) -> Result<Vec<Message>, BedrockError> {
    let Some(last) = messages.pop() else {
        return Ok(messages);
    };
    let mut content = last.content().to_vec();
    content.push(ContentBlock::CachePoint(cache_point(ttl)?));
    let rebuilt = Message::builder()
        .role(last.role().clone())
        .set_content(Some(content))
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))?;
    messages.push(rebuilt);
    Ok(messages)
}

/// Concatenate the text blocks of an assistant message.
pub(crate) fn collect_text(message: &Message) -> String {
    message
        .content()
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Extract per-turn usage, preserving absence.
///
/// The AWS JSON deserializer may materialize a default all-zero usage
/// structure when the service omits the block. A successful Converse call
/// necessarily consumes tokens, so an all-zero shape is reported as `None`
/// rather than a misleading metered zero — callers render absence.
pub(crate) fn optional_usage(
    usage: Option<&TokenUsage>,
    model_id: &str,
    cache_ttl: Option<CacheTtlChoice>,
) -> Option<TurnUsage> {
    usage.and_then(|usage| {
        let extracted = tokens::extract_turn_usage(usage, model_id, cache_ttl);
        (extracted.input_tokens > 0
            || extracted.output_tokens > 0
            || extracted.cache_read_input_tokens > 0
            || extracted.cache_write_input_tokens > 0)
            .then_some(extracted)
    })
}

/// Enforce a complete text response.
///
/// `EndTurn` and `StopSequence` are complete; `MaxTokens` and
/// `ModelContextWindowExceeded` become typed errors so no caller can
/// persist silently truncated output as success. Any other stop reason is a
/// protocol surprise for a text-only flow.
pub(crate) fn ensure_complete_text_response(
    stop_reason: &StopReason,
    max_output_tokens: u32,
) -> Result<(), BedrockError> {
    match stop_reason {
        StopReason::EndTurn | StopReason::StopSequence => Ok(()),
        StopReason::MaxTokens => Err(BedrockError::ResponseTruncated { max_output_tokens }),
        StopReason::ModelContextWindowExceeded => Err(BedrockError::ContextWindowExceeded),
        other => Err(BedrockError::ResponseParse(format!(
            "unexpected stop reason {other:?} for a text-only response"
        ))),
    }
}

/// The completed result of a streamed Converse call.
#[derive(Debug)]
pub struct StreamOutcome {
    /// The full accumulated assistant text.
    pub text: String,
    /// The wire stop reason (e.g. `end_turn`), for the caller's terminal
    /// stream event. Always a complete-response reason — truncation and
    /// overflow already surfaced as errors in [`StreamCollector::finish`].
    pub stop_reason: String,
    /// Per-turn usage from the trailing `metadata` event; `None` preserved
    /// when the service omitted it (see [`optional_usage`]).
    pub usage: Option<TurnUsage>,
}

/// Accumulates a `ConverseStream` response: text deltas, the stop reason
/// from `messageStop`, and usage from the trailing `metadata` event.
///
/// Pure event-folding, so tests can drive it with synthetic events; the
/// terminal [`Self::finish`] applies the same truncation and stop-reason
/// rules as the unary path.
#[derive(Debug, Default)]
pub struct StreamCollector {
    text: String,
    stop_reason: Option<StopReason>,
    usage: Option<TokenUsage>,
}

impl StreamCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Absorb one stream event, returning the text delta it carried (if any)
    /// so the caller can forward it incrementally.
    pub fn absorb(&mut self, event: ConverseStreamOutput) -> Option<String> {
        match event {
            ConverseStreamOutput::ContentBlockDelta(event) => match event.delta {
                Some(ContentBlockDelta::Text(text)) => {
                    self.text.push_str(&text);
                    Some(text)
                }
                _ => None,
            },
            ConverseStreamOutput::MessageStop(event) => {
                self.stop_reason = Some(event.stop_reason);
                None
            }
            ConverseStreamOutput::Metadata(event) => {
                self.usage = event.usage;
                None
            }
            _ => None,
        }
    }

    /// Close out the stream: a missing `messageStop` is a protocol error,
    /// truncation and context overflow become the same typed errors as the
    /// unary path, and an omitted usage block stays `None`. `cache_ttl` is
    /// the TTL the request's cache points carried (`None` when uncached),
    /// recorded on the usage so cache writes price at the right tier.
    pub fn finish(
        self,
        model_id: &str,
        max_output_tokens: u32,
        cache_ttl: Option<CacheTtlChoice>,
    ) -> Result<StreamOutcome, BedrockError> {
        let stop_reason = self.stop_reason.ok_or_else(|| {
            BedrockError::ResponseParse(
                "the response stream ended without a messageStop event".to_string(),
            )
        })?;
        ensure_complete_text_response(&stop_reason, max_output_tokens)?;
        let usage = optional_usage(self.usage.as_ref(), model_id, cache_ttl);
        Ok(StreamOutcome {
            text: self.text,
            stop_reason: stop_reason.as_str().to_string(),
            usage,
        })
    }
}

/// Structured cache/usage observability line for one completed Converse
/// call, shared by the chat and report flows. `hit_rate` is the fraction of
/// input tokens served from cache; `cache_ttl` names the write tier so a
/// console export answers "did caching hit, and at which TTL?". Fields are
/// model IDs, counts, and rates — never prompt or response content.
pub(crate) fn log_turn_usage(
    operation: &'static str,
    model_id: &str,
    usage: Option<&TurnUsage>,
) {
    if let Some(usage) = usage {
        let total_input =
            usage.input_tokens + usage.cache_read_input_tokens + usage.cache_write_input_tokens;
        let hit_rate = if total_input > 0 {
            usage.cache_read_input_tokens as f64 / total_input as f64
        } else {
            0.0
        };
        tracing::info!(
            target: "claria_bedrock::cache",
            operation,
            model_id,
            input_tokens = usage.input_tokens,
            cache_read = usage.cache_read_input_tokens,
            cache_write = usage.cache_write_input_tokens,
            cache_ttl = usage
                .cache_ttl
                .map_or("none", claria_core::model_id::CacheTtlChoice::as_str),
            hit_rate,
            cost_usd = usage.cost_usd,
            "turn complete"
        );
    } else {
        tracing::warn!(operation, model_id, "turn completed without a usage block");
    }
}

/// Incremental input-token budget shared by the chat and report flows.
///
/// Token counts and character measures are kept together so a request is
/// counted with a real `CountTokens` call only when needed: appended content
/// is estimated at ~4 characters per token, and the estimate is re-verified
/// against the service only once it comes within ~10% of the budget.
pub(crate) struct InputTokenBudget {
    budget: u32,
    /// `(verified_tokens, chars_at_verification)` from the last real count.
    verified: Option<(u32, u64)>,
    /// Whether the first check must run a real count (report semantics:
    /// count once per turn) or may trust the character estimate until it
    /// nears the budget (chat semantics).
    verify_first: bool,
}

impl InputTokenBudget {
    /// A budget whose first check runs a real `CountTokens` call, then
    /// estimates increments.
    pub(crate) fn exact(budget: u32) -> Self {
        Self {
            budget,
            verified: None,
            verify_first: true,
        }
    }

    /// A budget that trusts the character estimate until it comes within
    /// ~10% of the budget, only then verifying with a real count.
    pub(crate) fn estimated(budget: u32) -> Self {
        Self {
            budget,
            verified: None,
            verify_first: false,
        }
    }

    /// Check a request of `current_chars` characters against the budget.
    ///
    /// `count` runs a real `CountTokens` for the current request shape;
    /// `over_budget` builds the caller's typed error from
    /// `(input_tokens, budget)`.
    pub(crate) async fn ensure_within<F, Fut, E>(
        &mut self,
        current_chars: u64,
        count: F,
        over_budget: E,
    ) -> Result<(), BedrockError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<u32, BedrockError>>,
        E: FnOnce(u32, u32) -> BedrockError,
    {
        let estimate = match self.verified {
            Some((tokens, chars)) => tokens.saturating_add(
                u32::try_from(current_chars.saturating_sub(chars) / 4).unwrap_or(u32::MAX),
            ),
            None if self.verify_first => u32::MAX,
            None => u32::try_from(current_chars / 4).unwrap_or(u32::MAX),
        };
        // Within 90% of the budget (or a mandatory first check): verify.
        if u64::from(estimate) * 10 < u64::from(self.budget) * 9 {
            return Ok(());
        }
        let tokens = count().await?;
        self.verified = Some((tokens, current_chars));
        if tokens > self.budget {
            Err(over_budget(tokens, self.budget))
        } else {
            Ok(())
        }
    }
}

/// Convert a `serde_json::Value` to the Smithy `Document` the Converse tool
/// APIs speak.
pub(crate) fn json_to_document(value: &serde_json::Value) -> Result<Document, BedrockError> {
    match value {
        serde_json::Value::Null => Ok(Document::Null),
        serde_json::Value::Bool(value) => Ok(Document::Bool(*value)),
        serde_json::Value::String(value) => Ok(Document::String(value.clone())),
        serde_json::Value::Array(values) => values
            .iter()
            .map(json_to_document)
            .collect::<Result<Vec<_>, _>>()
            .map(Document::Array),
        serde_json::Value::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), json_to_document(value)?)))
            .collect::<Result<HashMap<_, _>, BedrockError>>()
            .map(Document::Object),
        serde_json::Value::Number(value) => {
            let number = if let Some(value) = value.as_u64() {
                Number::PosInt(value)
            } else if let Some(value) = value.as_i64() {
                Number::NegInt(value)
            } else if let Some(value) = value.as_f64() {
                if !value.is_finite() {
                    return Err(BedrockError::Serialization(serde_json::Error::io(
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "non-finite JSON number",
                        ),
                    )));
                }
                Number::Float(value)
            } else {
                return Err(BedrockError::SchemaViolation(
                    "JSON number could not be represented for Bedrock".to_string(),
                ));
            };
            Ok(Document::Number(number))
        }
    }
}

/// Convert a Smithy `Document` (tool-call input) back into JSON.
pub(crate) fn document_to_json(document: &Document) -> Result<serde_json::Value, BedrockError> {
    match document {
        Document::Null => Ok(serde_json::Value::Null),
        Document::Bool(value) => Ok(serde_json::Value::Bool(*value)),
        Document::String(value) => Ok(serde_json::Value::String(value.clone())),
        Document::Array(values) => values
            .iter()
            .map(document_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        Document::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), document_to_json(value)?)))
            .collect::<Result<serde_json::Map<_, _>, BedrockError>>()
            .map(serde_json::Value::Object),
        Document::Number(Number::PosInt(value)) => Ok(serde_json::Value::Number((*value).into())),
        Document::Number(Number::NegInt(value)) => Ok(serde_json::Value::Number((*value).into())),
        Document::Number(Number::Float(value)) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .ok_or_else(|| {
                BedrockError::ResponseParse(
                    "Bedrock document contained a non-finite number".to_string(),
                )
            }),
    }
}

/// Count the input tokens of one Converse-shaped request against a bare
/// foundation model ID (`CountTokens` rejects inference-profile IDs).
pub(crate) async fn count_input_tokens(
    client: &aws_sdk_bedrockruntime::Client,
    model_id: &str,
    request: ConverseTokensRequest,
) -> Result<u32, BedrockError> {
    let response = client
        .count_tokens()
        .model_id(model_id)
        .input(CountTokensInput::Converse(request))
        .send()
        .await
        .map_err(|error| classify_error("CountTokens", error))?;
    Ok(u32::try_from(response.input_tokens()).unwrap_or(u32::MAX))
}
