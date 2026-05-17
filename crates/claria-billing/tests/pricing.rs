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

/// Phase 2: cache prices are non-zero on caching-supported families.
#[test]
fn cache_prices_match_phase_2_table() {
    let sonnet = lookup("us.anthropic.claude-sonnet-4-20250514-v1:0").unwrap();
    assert!((sonnet.cache_read_per_million - 0.30).abs() < f64::EPSILON);
    assert!((sonnet.cache_write_per_million - 3.75).abs() < f64::EPSILON);

    let opus = lookup("us.anthropic.claude-opus-4-20250514-v1:0").unwrap();
    assert!((opus.cache_read_per_million - 1.50).abs() < f64::EPSILON);
    assert!((opus.cache_write_per_million - 18.75).abs() < f64::EPSILON);

    let haiku = lookup("us.anthropic.claude-3-5-haiku-20241022-v1:0").unwrap();
    assert!((haiku.cache_read_per_million - 0.08).abs() < f64::EPSILON);
    assert!((haiku.cache_write_per_million - 1.00).abs() < f64::EPSILON);
}

// ---------------------------------------------------------------------------
// Phase 3: Claude 4.5+ generation
// ---------------------------------------------------------------------------

/// Opus 4.5 is 3× cheaper than Opus 4 on input. Without this row, the
/// matcher's dash-boundary heuristic correctly refuses to confuse 4.5
/// with 4 — but then leaves us with no pricing at all (a regression that
/// would surface as "cost unavailable" on the current flagship model).
#[test]
fn lookup_opus_4_5() {
    let p = lookup("us.anthropic.claude-opus-4-5-20251101-v1:0")
        .expect("Opus 4.5 should resolve");
    assert!((p.input_per_million - 5.0).abs() < f64::EPSILON);
    assert!((p.output_per_million - 25.0).abs() < f64::EPSILON);
    assert!((p.cache_read_per_million - 0.50).abs() < f64::EPSILON);
    assert!((p.cache_write_per_million - 6.25).abs() < f64::EPSILON);
}

#[test]
fn lookup_opus_4_6_and_4_7() {
    let p46 = lookup("us.anthropic.claude-opus-4-6-20260201-v1:0")
        .expect("Opus 4.6 should resolve");
    let p47 = lookup("us.anthropic.claude-opus-4-7-20260401-v1:0")
        .expect("Opus 4.7 should resolve");
    // Same generation, same rate.
    assert!((p46.input_per_million - 5.0).abs() < f64::EPSILON);
    assert!((p47.input_per_million - 5.0).abs() < f64::EPSILON);
}

#[test]
fn lookup_sonnet_4_5_and_4_6() {
    let p45 = lookup("us.anthropic.claude-sonnet-4-5-20251015-v1:0")
        .expect("Sonnet 4.5 should resolve");
    let p46 = lookup("us.anthropic.claude-sonnet-4-6-20260301-v1:0")
        .expect("Sonnet 4.6 should resolve");
    assert!((p45.input_per_million - 3.0).abs() < f64::EPSILON);
    assert!((p46.input_per_million - 3.0).abs() < f64::EPSILON);
    assert!((p45.output_per_million - 15.0).abs() < f64::EPSILON);
}

#[test]
fn lookup_haiku_4_5() {
    let p = lookup("us.anthropic.claude-haiku-4-5-20251001-v1:0")
        .expect("Haiku 4.5 should resolve");
    assert!((p.input_per_million - 1.0).abs() < f64::EPSILON);
    assert!((p.output_per_million - 5.0).abs() < f64::EPSILON);
}

/// Opus 4 and Opus 4.5 must price differently — confirms the table
/// actually has two distinct entries and that the dash-boundary matcher
/// is keeping them apart.
#[test]
fn opus_4_and_4_5_have_distinct_pricing() {
    let p4 = lookup("us.anthropic.claude-opus-4-20250514-v1:0").unwrap();
    let p45 = lookup("us.anthropic.claude-opus-4-5-20251101-v1:0").unwrap();
    assert!(
        (p4.input_per_million - p45.input_per_million).abs() > 1.0,
        "Opus 4 ($15) and Opus 4.5 ($5) must price differently"
    );
}
