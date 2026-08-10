//! Shared Bedrock Converse plumbing for every claria-bedrock flow.
//!
//! One owner for runtime-client construction, structured service-error
//! classification, response-text collection, optional usage extraction, and
//! the `CountTokens` wrapper. Chat, extraction, translation, and the report
//! writer all speak to Bedrock through this module so their error and usage
//! semantics cannot drift apart.

use aws_sdk_bedrockruntime::{
    error::{DisplayErrorContext, ProvideErrorMetadata, SdkError},
    operation::RequestId,
    types::{ContentBlock, ConverseTokensRequest, CountTokensInput, Message, TokenUsage},
};
use claria_core::models::turn_usage::TurnUsage;

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
pub(crate) fn optional_usage(usage: Option<&TokenUsage>, model_id: &str) -> Option<TurnUsage> {
    usage.and_then(|usage| {
        let extracted = tokens::extract_turn_usage(usage, model_id);
        (extracted.input_tokens > 0
            || extracted.output_tokens > 0
            || extracted.cache_read_input_tokens > 0
            || extracted.cache_write_input_tokens > 0)
            .then_some(extracted)
    })
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
