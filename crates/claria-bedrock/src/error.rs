use claria_core::provider_diagnostic::ProviderMessage;
use thiserror::Error;

/// Which of a streamed call's two waits ended it.
///
/// The two are different failures with different remedies, and a reader told
/// to raise the wrong one is worse off than one told nothing: the response
/// that never started is a cold prefill, and the response that stopped
/// mid-flight is a model composing something too large to emit inside one
/// silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamInterruption {
    /// The service never sent a first frame inside the first-frame wait.
    NeverStarted,
    /// A stream already under way went quiet past the idle wait.
    WentSilent,
    /// The stream failed outright mid-response — not a timeout.
    Dropped,
}

impl StreamInterruption {
    /// Whether a longer wait is the thing that would have saved this call.
    pub fn is_timeout(self) -> bool {
        matches!(self, Self::NeverStarted | Self::WentSilent)
    }
}

#[derive(Debug, Error)]
pub enum BedrockError {
    #[error("model invocation failed: {0}")]
    Invocation(String),

    /// The response stream stalled or dropped before `messageStop`. Nothing
    /// from the call is committed, so re-sending the identical request is
    /// safe — the writer loop retries on exactly this variant.
    #[error("{message}")]
    StreamInterrupted {
        operation: &'static str,
        kind: StreamInterruption,
        message: String,
    },

    #[error("response parsing failed: {0}")]
    ResponseParse(String),

    #[error("response did not conform to expected schema: {0}")]
    SchemaViolation(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("model not supported: {0}")]
    UnsupportedModel(String),

    #[error(
        "The Bedrock {operation} request failed: {message} (code {code}, status {status:?}, request ID {request_id:?})"
    )]
    Service {
        operation: &'static str,
        status: Option<u16>,
        code: String,
        request_id: Option<String>,
        message: ProviderMessage,
    },

    /// No service response was received. This SDK chain contains transport
    /// diagnostics rather than provider prose or document content.
    #[error("The Bedrock {operation} request could not reach the service: {message}")]
    Transport {
        operation: &'static str,
        message: String,
    },

    #[error(
        "report input uses {input_tokens} tokens, exceeding the {input_token_budget}-token budget for {model_id}"
    )]
    ContextBudgetExceeded {
        model_id: String,
        input_tokens: u32,
        input_token_budget: u32,
    },

    #[error(
        "The model hit its {max_output_tokens}-token response limit before finishing, so the incomplete response was discarded. Retry with a smaller request."
    )]
    ResponseTruncated { max_output_tokens: u32 },

    #[error(
        "The conversation and selected records exceed this model's context window. Shorten the conversation or deselect records, then try again."
    )]
    ContextWindowExceeded,

    #[error("AWS config error: {0}")]
    Config(String),

    /// The reader pressed Stop while the request was in flight. Not a
    /// failure — the caller decides what its partial work is worth: a stopped
    /// chat turn keeps the partial text as the answer, while a stopped
    /// structured call produced nothing and keeps nothing. Callers that cannot
    /// be stopped never construct it.
    #[error("the request was stopped by the user")]
    Stopped,
}

impl BedrockError {
    /// Diagnostic text safe for support logs. Provider prose is rendered by
    /// `ProviderMessage`; protocol and builder errors can contain model or
    /// document text, so those use fixed labels.
    pub fn log_diagnostic(&self) -> String {
        match self {
            Self::Service { .. }
            | Self::Transport { .. }
            | Self::StreamInterrupted { .. }
            | Self::ContextBudgetExceeded { .. }
            | Self::ResponseTruncated { .. }
            | Self::ContextWindowExceeded
            | Self::Stopped => self.to_string(),
            Self::SchemaViolation(_) | Self::ResponseParse(_) => {
                format!("Bedrock protocol failure: {}", self.protocol_failure_kind())
            }
            _ => "Bedrock request construction or configuration failed".to_string(),
        }
    }

    /// A fixed log label for protocol failures that may include model output
    /// in their detailed error string.
    pub fn protocol_failure_kind(&self) -> &'static str {
        match self {
            Self::ResponseParse(_) => "response_parse",
            Self::SchemaViolation(message) if message.starts_with("unknown report tool:") => {
                "unknown_tool"
            }
            Self::SchemaViolation(message) if message.contains("duplicate") => "duplicate_tool",
            Self::SchemaViolation(message) if message.contains("tool-use limit") => {
                "tool_use_limit"
            }
            Self::SchemaViolation(message) if message.contains("cache plan") => "cache_plan",
            Self::SchemaViolation(_) => "schema_violation",
            _ => "other",
        }
    }

    /// True for failures where the request is known not to have completed:
    /// the response stream broke before `messageStop`, or the request never
    /// received a response at all (connect, dispatch, and client-timeout
    /// failures, which [`classify_error`](crate::converse) labels as transport
    /// failures). Nothing from such a call is committed, so
    /// re-sending the identical request is safe.
    /// Which wait ran out, when one did. `None` for interruptions that are
    /// not a timeout at all.
    pub fn stream_interruption(&self) -> Option<StreamInterruption> {
        match self {
            Self::StreamInterrupted { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    pub fn is_interrupted_before_completion(&self) -> bool {
        matches!(
            self,
            Self::StreamInterrupted { .. } | Self::Transport { .. }
        )
    }

    /// True when Bedrock is asking the caller to slow down or come back,
    /// rather than reporting anything wrong with the request: the same
    /// bytes sent again later can succeed.
    ///
    /// Covers `ThrottlingException` (rate limit), `ServiceUnavailableException`
    /// and HTTP 503 (capacity), `ModelNotReadyException` (a profile still
    /// warming), and HTTP 429 for services that throttle by status without a
    /// recognised code.
    ///
    /// `ServiceQuotaExceededException` is deliberately absent. An account
    /// quota does not clear on a backoff timer, so retrying it burns the
    /// user's time and hides the one error whose fix is a quota increase.
    pub fn is_retryable_throttle(&self) -> bool {
        let Self::Service { code, status, .. } = self else {
            return false;
        };
        matches!(
            code.as_str(),
            "ThrottlingException" | "ServiceUnavailableException" | "ModelNotReadyException"
        ) || matches!(status, Some(429 | 503))
    }
}
