use claria_core::provider_diagnostic::{
    ProviderMessage, message_field, message_reason, safe_request_id, safe_service_code,
};

#[test]
fn real_bedrock_validation_wording_has_actionable_safe_reasons() {
    let cases = [
        (
            "Invocation of model ID anthropic.claude-opus-4-6-v1 with on-demand throughput isn’t supported. Retry your request with the ID or ARN of an inference profile that contains this model.",
            "requires_inference_profile",
            "model_id",
        ),
        (
            "You don't have access to the model with the specified model ID.",
            "access_denied",
            "model_id",
        ),
        (
            "The provided model identifier is invalid.",
            "invalid_model_identifier",
            "model_id",
        ),
        (
            "Input is too long for requested model.",
            "input_too_long",
            "unknown",
        ),
        (
            "Too many tokens, please wait before trying again.",
            "token_rate_limited",
            "unknown",
        ),
        (
            "A conversation must start with a user message.",
            "invalid_conversation_role",
            "unknown",
        ),
        (
            "The format of the value at messages.0.content.1.document.name is invalid. Jane Doe intake.docx",
            "invalid_document_name",
            "document.name",
        ),
    ];
    for (raw, reason, field) in cases {
        assert_eq!(message_reason(raw), reason, "{raw}");
        assert_eq!(message_field(raw), field, "{raw}");
        let safe = ProviderMessage::new(raw);
        assert!(!safe.to_string().contains("Jane Doe"));
        assert!(!format!("{safe:?}").contains("Jane Doe"));
        assert_eq!(safe.reason(), reason);
    }
}

#[test]
fn unknown_provider_prose_never_reaches_standard_formatters() {
    let message = ProviderMessage::new("Jane Doe's report says XYZ");
    assert_eq!(message.raw(), "Jane Doe's report says XYZ");
    assert_eq!(
        message.to_string(),
        "the provider rejected the request (reason unclassified)"
    );
    assert_eq!(
        format!("{message:?}"),
        "ProviderMessage { reason: \"unclassified\", field: \"unknown\" }"
    );
}

#[test]
fn metadata_shape_checks_accept_aws_codes_and_s3_request_ids() {
    for code in [
        "ValidationException",
        "ModelTimeoutException",
        "RequestTimeTooSkewed",
        "InternalFailureException",
        "InvalidClientTokenId",
    ] {
        assert_eq!(safe_service_code(code), code);
    }
    for request_id in [
        "TJB5XE1VNSK6BA8D",
        "318BC8BC148832E5",
        "fff43026-69a2-4747-b46b-a9dca5ea3c04",
    ] {
        assert_eq!(safe_request_id(request_id), request_id);
    }
    assert_eq!(
        safe_service_code("ValidationException\npatient=Jane"),
        "withheld"
    );
    assert_eq!(safe_request_id("Jane Doe intake.docx"), "withheld");
}
