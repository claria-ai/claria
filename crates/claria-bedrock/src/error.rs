use thiserror::Error;

#[derive(Debug, Error)]
pub enum BedrockError {
    #[error("model invocation failed: {0}")]
    Invocation(String),

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
