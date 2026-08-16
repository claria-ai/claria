use thiserror::Error;

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

    #[error("model agreement error: {0}")]
    Agreement(String),
}

impl BedrockError {
    /// True for failures where the request is known not to have completed:
    /// the response stream broke before `messageStop`, or the request never
    /// received a response at all (connect, dispatch, and client-timeout
    /// failures, which [`classify_error`](crate::converse) labels
    /// `DispatchFailure`). Nothing from such a call is committed, so
    /// re-sending the identical request is safe.
    pub fn is_interrupted_before_completion(&self) -> bool {
        match self {
            Self::StreamInterrupted { .. } => true,
            Self::Service { code, .. } => code == "DispatchFailure",
            _ => false,
        }
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
