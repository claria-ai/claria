//! Unit tests for the pure helpers in `claria_bedrock::agreements` and the
//! Converse error classifier. These do not touch AWS.

use claria_bedrock::agreements::{
    classify_axes, use_case_form_from_bytes, use_case_form_to_bytes, EnrollmentStatus, UseCaseForm,
};
use claria_bedrock::chat::classify_converse_error;
use claria_bedrock::error::{BedrockError, ModelAccessReason};

// ── classify_axes ─────────────────────────────────────────────────────────

#[test]
fn region_unavailable_short_circuits() {
    let s = classify_axes("NOT_AVAILABLE", "AVAILABLE", Some("AVAILABLE"), None, "AUTHORIZED");
    assert_eq!(s, EnrollmentStatus::RegionUnavailable);
}

#[test]
fn entitlement_available_is_executed() {
    let s = classify_axes("AVAILABLE", "AVAILABLE", Some("NOT_AVAILABLE"), None, "AUTHORIZED");
    assert_eq!(s, EnrollmentStatus::Executed);
}

#[test]
fn agreement_available_is_available() {
    let s = classify_axes("AVAILABLE", "NOT_AVAILABLE", Some("AVAILABLE"), None, "AUTHORIZED");
    assert_eq!(s, EnrollmentStatus::Available);
}

#[test]
fn agreement_pending_is_pending() {
    let s = classify_axes("AVAILABLE", "NOT_AVAILABLE", Some("PENDING"), None, "AUTHORIZED");
    assert_eq!(s, EnrollmentStatus::Pending);
}

#[test]
fn agreement_error_is_blocked_with_reason() {
    let s = classify_axes(
        "AVAILABLE",
        "NOT_AVAILABLE",
        Some("ERROR"),
        Some("subscription failed"),
        "AUTHORIZED",
    );
    assert_eq!(
        s,
        EnrollmentStatus::Blocked {
            reason: "subscription failed".to_string()
        }
    );
}

#[test]
fn no_agreement_when_authorized_is_blocked_no_offer() {
    let s = classify_axes("AVAILABLE", "NOT_AVAILABLE", Some("NOT_AVAILABLE"), None, "AUTHORIZED");
    match s {
        EnrollmentStatus::Blocked { reason } => assert!(reason.contains("no marketplace agreement")),
        other => panic!("expected Blocked, got {other:?}"),
    }
}

#[test]
fn no_agreement_when_unauthorized_is_blocked_not_authorized() {
    let s = classify_axes("AVAILABLE", "NOT_AVAILABLE", None, None, "NOT_AUTHORIZED");
    match s {
        EnrollmentStatus::Blocked { reason } => assert!(reason.contains("isn't authorized")),
        other => panic!("expected Blocked, got {other:?}"),
    }
}

// ── classify_converse_error ─────────────────────────────────────────────────

#[test]
fn converse_ftu_error_maps_to_use_case_form_required() {
    let e = classify_converse_error(
        "us.anthropic.claude-opus-4-6-v1:0",
        "FTUFormNotFilled: Model use case details have not been submitted for this account",
    );
    match e {
        BedrockError::ModelAccess { model_id, reason } => {
            assert_eq!(model_id, "anthropic.claude-opus-4-6-v1:0");
            assert_eq!(reason, ModelAccessReason::UseCaseFormRequired);
        }
        other => panic!("expected ModelAccess, got {other:?}"),
    }
}

#[test]
fn converse_on_demand_error_maps_to_needs_inference_profile() {
    let e = classify_converse_error(
        "anthropic.claude-opus-4-6-v1:0",
        "Invocation of model ID anthropic.claude-opus-4-6-v1:0 with on-demand throughput isn't supported",
    );
    match e {
        BedrockError::ModelAccess { reason, .. } => {
            assert_eq!(reason, ModelAccessReason::NeedsInferenceProfile);
        }
        other => panic!("expected ModelAccess, got {other:?}"),
    }
}

#[test]
fn converse_access_denied_maps_to_not_subscribed() {
    let e = classify_converse_error(
        "us.anthropic.claude-opus-4-6-v1:0",
        "AccessDeniedException: You don't have access to the model with the specified model ID.",
    );
    match e {
        BedrockError::ModelAccess { model_id, reason } => {
            assert_eq!(model_id, "anthropic.claude-opus-4-6-v1:0");
            assert_eq!(reason, ModelAccessReason::NotSubscribed);
        }
        other => panic!("expected ModelAccess, got {other:?}"),
    }
}

#[test]
fn converse_unrelated_error_stays_invocation() {
    let e = classify_converse_error("us.anthropic.claude-opus-4-6-v1:0", "ThrottlingException: slow down");
    assert!(matches!(e, BedrockError::Invocation(_)));
}

// ── use-case form blob round-trip ───────────────────────────────────────────

#[test]
fn use_case_form_blob_uses_camel_case_and_round_trips() {
    let form = UseCaseForm {
        company_name: "Acme Health".to_string(),
        company_website: "https://acme.example".to_string(),
        intended_users: 0,
        industry_option: "Healthcare".to_string(),
        other_industry_option: None,
        use_cases: "Clinical note drafting".to_string(),
    };

    let bytes = use_case_form_to_bytes(&form).expect("serialize");
    let json = String::from_utf8(bytes.clone()).expect("utf8");

    // AWS expects camelCase keys on the blob, not the codebase-default snake_case.
    assert!(json.contains("\"companyName\""), "got: {json}");
    assert!(json.contains("\"intendedUsers\""), "got: {json}");
    assert!(!json.contains("\"company_name\""), "got: {json}");

    let parsed = use_case_form_from_bytes(&bytes).expect("round-trip parse");
    assert_eq!(parsed, form);
}

#[test]
fn use_case_form_from_garbage_is_none() {
    assert!(use_case_form_from_bytes(b"not json").is_none());
}
