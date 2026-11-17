//! Safe rendering of provider errors that may echo patient or document text.
//!
//! The raw message remains available through an explicit accessor. Ordinary
//! `Display` and `Debug` output uses only Claria-owned, fixed text so logging
//! an error cannot accidentally copy provider prose into a support export.

use std::fmt;

/// Provider prose that may contain PHI. Both standard formatters redact it.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderMessage(String);

impl ProviderMessage {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Use only where the original response is required; never log this value.
    pub fn raw(&self) -> &str {
        &self.0
    }

    pub fn reason(&self) -> &'static str {
        message_reason(&self.0)
    }

    pub fn field(&self) -> &'static str {
        message_field(&self.0)
    }
}

impl From<String> for ProviderMessage {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ProviderMessage {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl fmt::Display for ProviderMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(message_summary(&self.0))
    }
}

impl fmt::Debug for ProviderMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderMessage")
            .field("reason", &self.reason())
            .field("field", &self.field())
            .finish()
    }
}

/// A fixed diagnostic category. Match actual AWS wording as well as the
/// document-field paths returned by Bedrock validation errors.
pub fn message_reason(message: &str) -> &'static str {
    let lower = message.to_lowercase().replace('’', "'");
    if lower.contains("on-demand throughput")
        && (lower.contains("not supported") || lower.contains("isn't supported"))
    {
        "requires_inference_profile"
    } else if lower.contains("don't have access to the model")
        || lower.contains("not authorized")
        || lower.contains("access denied")
    {
        "access_denied"
    } else if lower.contains("model identifier is invalid")
        || lower.contains("invalid model identifier")
    {
        "invalid_model_identifier"
    } else if lower.contains("input is too long") || lower.contains("context window") {
        "input_too_long"
    } else if lower.contains("too many tokens") || lower.contains("rate limit") {
        "token_rate_limited"
    } else if lower.contains("conversation must start with a user message") {
        "invalid_conversation_role"
    } else if lower.contains("document.name")
        || lower.contains("document name ")
        || lower.contains("name of the document")
    {
        "invalid_document_name"
    } else if lower.contains("unsupported document format")
        || lower.contains("document format is not supported")
        || lower.contains("invalid document format")
        || lower.contains("document.format")
    {
        "unsupported_document_format"
    } else if lower.contains("document")
        && (lower.contains("too large") || lower.contains("size limit"))
    {
        "document_too_large"
    } else if lower.contains("document")
        && (lower.contains("not valid") || lower.contains("invalid") || lower.contains("corrupt"))
    {
        "invalid_document"
    } else if lower.contains("model") && lower.contains("not supported") {
        "unsupported_model"
    } else if lower.contains("model") && lower.contains("not found") {
        "model_not_found"
    } else if lower.contains("region") && lower.contains("not supported") {
        "unsupported_region"
    } else if lower.contains("quota") {
        "quota_exceeded"
    } else {
        "unclassified"
    }
}

/// A useful sentence built entirely from fixed strings owned by Claria.
pub fn message_summary(message: &str) -> &'static str {
    match message_reason(message) {
        "requires_inference_profile" => {
            "the model requires an inference profile for this invocation"
        }
        "access_denied" => "the account lacks access to the requested model or service",
        "invalid_model_identifier" => "the model identifier is invalid",
        "input_too_long" => "the request exceeds the model's input limit",
        "token_rate_limited" => "the request exceeded the token rate limit",
        "invalid_conversation_role" => "the conversation must start with a user message",
        "invalid_document_name" => "the document name is invalid",
        "unsupported_document_format" => "the document format is unsupported",
        "document_too_large" => "the document exceeds the size limit",
        "invalid_document" => "the document is invalid or corrupt",
        "unsupported_model" => "the selected model is unsupported",
        "model_not_found" => "the selected model was not found",
        "unsupported_region" => "the selected region is unsupported",
        "quota_exceeded" => "the service quota was exceeded",
        _ => "the provider rejected the request (reason unclassified)",
    }
}

/// Emit only a recognized request-field label, never a dynamic path or index.
pub fn message_field(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("document.source.bytes") {
        "document.source.bytes"
    } else if lower.contains("document.format") {
        "document.format"
    } else if lower.contains("document.name") {
        "document.name"
    } else if lower.contains("max_tokens") || lower.contains("maxtokens") {
        "inference.max_tokens"
    } else if lower.contains("model identifier")
        || lower.contains("modelid")
        || lower.contains("model id")
    {
        "model_id"
    } else {
        "unknown"
    }
}

/// AWS supplies service codes as protocol metadata. Check their shape to
/// prevent log-line injection without losing new service codes.
pub fn safe_service_code(code: &str) -> &str {
    let mut bytes = code.bytes();
    if (1..=64).contains(&code.len())
        && bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'.')
    {
        code
    } else {
        "withheld"
    }
}

/// AWS request IDs include UUIDs and shorter alphanumeric S3 IDs.
pub fn safe_request_id(id: &str) -> &str {
    if (8..=128).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        id
    } else {
        "withheld"
    }
}
