//! Tests for `claria_billing::pricing` — verifies the lookup correctness
//! against the substring-match trap in the old implementation.

use claria_billing::pricing::{PRICING_VERSION, lookup};

#[test]
fn pricing_version_is_at_least_one() {
    // Phase 1 stamps version 1; later phases bump it.
    const _CHECK: () = assert!(PRICING_VERSION >= 1);
}

#[test]
fn lookup_us_inference_profile_for_opus_4() {
    let p = lookup("us.anthropic.claude-opus-4-20250514-v1:0").expect("Opus 4 should resolve");
    assert!((p.input_per_million - 15.0).abs() < f64::EPSILON);
    assert!((p.output_per_million - 75.0).abs() < f64::EPSILON);
}

#[test]
fn lookup_us_inference_profile_for_sonnet_4() {
    let p = lookup("us.anthropic.claude-sonnet-4-20250514-v1:0").expect("Sonnet 4 should resolve");
    assert!((p.input_per_million - 3.0).abs() < f64::EPSILON);
    assert!((p.output_per_million - 15.0).abs() < f64::EPSILON);
}

#[test]
fn lookup_haiku() {
    let p = lookup("us.anthropic.claude-3-5-haiku-20241022-v1:0").expect("Haiku should resolve");
    assert!((p.input_per_million - 0.80).abs() < f64::EPSILON);
    assert!((p.output_per_million - 4.0).abs() < f64::EPSILON);
}

#[test]
fn lookup_global_inference_profile() {
    let p = lookup("global.anthropic.claude-sonnet-4-20250514-v1:0")
        .expect("global. profile should also resolve");
    assert!((p.input_per_million - 3.0).abs() < f64::EPSILON);
}

#[test]
fn lookup_bare_foundation_id() {
    let p =
        lookup("anthropic.claude-opus-4-20250514-v1:0").expect("bare foundation id should resolve");
    assert!((p.input_per_million - 15.0).abs() < f64::EPSILON);
}

#[test]
fn lookup_unknown_returns_none() {
    assert!(lookup("anthropic.unknown-model-v1:0").is_none());
    assert!(lookup("us.amazon.titan-text-v1:0").is_none());
}

/// The substring-match trap: `id.contains("claude-opus-4")` would falsely
/// match a future `claude-opus-4-1` version. Our exact-prefix matcher must
/// reject it so we don't silently mis-price the new model.
#[test]
fn lookup_rejects_substring_collision_on_opus() {
    // Hypothetical future model — must NOT resolve to Opus 4 pricing.
    assert!(
        lookup("us.anthropic.claude-opus-4-1-future").is_none(),
        "claude-opus-4-1-future must NOT match the claude-opus-4 family stem"
    );
    assert!(
        lookup("anthropic.claude-opus-4-1-20260101-v1:0").is_none(),
        "claude-opus-4-1-* must NOT match the claude-opus-4 family stem"
    );
}

/// Same trap for Sonnet 4 vs a hypothetical Sonnet 4-1.
#[test]
fn lookup_rejects_substring_collision_on_sonnet() {
    assert!(
        lookup("us.anthropic.claude-sonnet-4-1-future").is_none(),
        "claude-sonnet-4-1-* must NOT match the claude-sonnet-4 family stem"
    );
}

/// Cache fields are zero in Phase 1 — Phase 2 fills them in.
#[test]
fn cache_fields_are_zero_in_phase_1() {
    let p = lookup("us.anthropic.claude-sonnet-4-20250514-v1:0").unwrap();
    assert_eq!(p.cache_read_per_million, 0.0);
    assert_eq!(p.cache_write_per_million, 0.0);
}
