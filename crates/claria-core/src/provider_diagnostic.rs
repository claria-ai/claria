//! Bounded, PHI-free summaries of untrusted provider error messages.
//!
//! A provider may echo a filename, prompt, transcript, or document text in
//! an error. Never send its free-form message to tracing or a support export.

use sha2::{Digest, Sha256};

/// A stable reason assembled only from strings controlled by Claria.
/// Unknown messages remain identifiable by fingerprint and request ID.
pub fn message_reason(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("unsupported document format")
        || lower.contains("document format is not supported")
        || lower.contains("invalid document format")
    {
        "unsupported_document_format"
    } else if lower.contains("document.name")
        || lower.contains("document name ")
        || lower.contains("name of the document")
    {
        "invalid_document_name"
    } else if lower.contains("document") && lower.contains("too large")
        || lower.contains("document") && lower.contains("size limit")
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
    } else if lower.contains("not authorized") || lower.contains("access denied") {
        "access_denied"
    } else if lower.contains("region") && lower.contains("not supported") {
        "unsupported_region"
    } else if lower.contains("quota") || lower.contains("rate limit") {
        "quota_or_rate_limit"
    } else {
        "unrecognized"
    }
}

/// The rejected request field, when the provider names one we recognize.
/// Emit a fixed label rather than copying its path with dynamic indices.
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
    } else if lower.contains("model identifier") || lower.contains("modelid") {
        "model_id"
    } else {
        "unknown"
    }
}

/// SHA-256 of the complete message, for correlating identical failures without
/// writing any portion of the provider's free-form text to a log.
pub fn message_sha256(message: &str) -> String {
    format!("{:x}", Sha256::digest(message.as_bytes()))
}

/// Only known AWS service codes may be written to a support log.
pub fn safe_service_code(code: &str) -> &str {
    const CODES: &[&str] = &[
        "AccessDenied",
        "AccessDeniedException",
        "BadRequestException",
        "ConditionalRequestConflict",
        "ConflictException",
        "DataUnavailableException",
        "DispatchFailure",
        "ExpiredToken",
        "InternalServerException",
        "InvalidAccessKeyId",
        "InvalidArgument",
        "InvalidParameterCombinationException",
        "InvalidParameterValueException",
        "LimitExceededException",
        "ModelNotReadyException",
        "NoSuchBucket",
        "NoSuchKey",
        "PreconditionFailed",
        "RequestTimeout",
        "ResourceNotFoundException",
        "ServiceQuotaExceededException",
        "ServiceUnavailableException",
        "SignatureDoesNotMatch",
        "SlowDown",
        "ThrottlingException",
        "UnknownServiceError",
        "UnrecognizedClientException",
        "ValidationException",
    ];
    if CODES.contains(&code) {
        code
    } else {
        "withheld"
    }
}

/// AWS request IDs used here are UUIDs or hexadecimal tokens. Refuse other
/// strings even when they contain no whitespace: a filename can look like an
/// opaque identifier too.
pub fn safe_request_id(id: &str) -> &str {
    if (20..=128).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        id
    } else {
        "withheld"
    }
}
