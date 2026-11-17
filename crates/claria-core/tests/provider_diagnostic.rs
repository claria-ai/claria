use claria_core::provider_diagnostic::{
    message_field, message_reason, message_sha256, safe_request_id, safe_service_code,
};

#[test]
fn recognizes_validation_reasons_without_echoing_patient_data() {
    let cases = [
        (
            "Unsupported document format in Jane Doe intake.docx",
            "unsupported_document_format",
        ),
        (
            "Document name Jane Doe intake.docx exceeds the allowed length",
            "invalid_document_name",
        ),
        (
            "messages.0.content.1.document.name: Jane Doe intake.docx is invalid",
            "invalid_document_name",
        ),
        (
            "The document named Jane Doe intake.docx is not valid",
            "invalid_document",
        ),
        (
            "Model access is not authorized for Jane Doe",
            "access_denied",
        ),
        ("Unexpected provider response for Jane Doe", "unrecognized"),
    ];
    for (message, expected) in cases {
        let reason = message_reason(message);
        assert_eq!(reason, expected);
        assert!(!reason.contains("Jane"));
        assert!(!reason.contains("Doe"));
        let hash = message_sha256(message);
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!hash.contains("Jane"));
    }
}

#[test]
fn only_known_codes_and_request_ids_can_enter_provider_log_fields() {
    assert_eq!(
        safe_service_code("ValidationException"),
        "ValidationException"
    );
    assert_eq!(safe_service_code("AccessDenied"), "AccessDenied");
    assert_eq!(
        safe_request_id("fff43026-69a2-4747-b46b-a9dca5ea3c04"),
        "fff43026-69a2-4747-b46b-a9dca5ea3c04"
    );
    assert_eq!(safe_service_code("JaneDoeException"), "withheld");
    assert_eq!(safe_request_id("Jane Doe"), "withheld");
    assert_eq!(safe_request_id("patient-record.docx"), "withheld");
    assert_eq!(safe_request_id(&"a".repeat(129)), "withheld");
}

#[test]
fn extracts_only_known_field_names() {
    assert_eq!(
        message_field("messages.0.content.1.document.source.bytes: Jane Doe is invalid"),
        "document.source.bytes"
    );
    assert_eq!(message_field("Unknown field for Jane Doe"), "unknown");
}
