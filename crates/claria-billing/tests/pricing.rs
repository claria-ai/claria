//! Tests for `claria_billing::pricing` — legacy families price at their
//! historical rates, everything else resolves by tier, and non-Claude
//! models return `None` so the UI omits cost.

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
fn lookup_opus_4_1_prices_as_legacy_generation() {
    let p = lookup("us.anthropic.claude-opus-4-1-20250805-v1:0").expect("Opus 4.1 should resolve");
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
fn lookup_haiku_3_5() {
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
    assert!(lookup("us.amazon.nova-pro-v1:0").is_none());
}

/// The legacy Opus 4 rate ($15) must not swallow later minors ($5): the
/// dash-boundary matcher keeps `claude-opus-4` and `claude-opus-4-1`
/// scoped to their dated IDs, and everything else in the tier falls
/// through to the current $5 rate.
#[test]
fn opus_legacy_rate_does_not_leak_to_newer_minors() {
    let p = lookup("us.anthropic.claude-opus-4-9-future").expect("tier match should resolve");
    assert!((p.input_per_million - 5.0).abs() < f64::EPSILON);
}

/// Cache prices follow the standard multipliers: 0.1× input for reads,
/// 1.25× input for writes.
#[test]
fn cache_prices_follow_standard_multipliers() {
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
// Current generations resolve by tier
// ---------------------------------------------------------------------------

#[test]
fn lookup_opus_4_5_through_4_8() {
    for id in [
        "us.anthropic.claude-opus-4-5-20251101-v1:0",
        "us.anthropic.claude-opus-4-6-20260201-v1:0",
        // The un-dated profile shape Bedrock actually serves for 4.6+.
        "us.anthropic.claude-opus-4-6-v1",
        "us.anthropic.claude-opus-4-7-v1",
        "us.anthropic.claude-opus-4-8-v1",
    ] {
        let p = lookup(id).unwrap_or_else(|| panic!("{id} should resolve"));
        assert!((p.input_per_million - 5.0).abs() < f64::EPSILON, "{id}");
        assert!((p.output_per_million - 25.0).abs() < f64::EPSILON, "{id}");
        assert!(
            (p.cache_read_per_million - 0.50).abs() < f64::EPSILON,
            "{id}"
        );
        assert!(
            (p.cache_write_per_million - 6.25).abs() < f64::EPSILON,
            "{id}"
        );
    }
}

#[test]
fn lookup_fable_and_mythos() {
    for id in [
        "anthropic.claude-fable-5",
        "us.anthropic.claude-fable-5-v1:0",
        "us.anthropic.claude-mythos-5-v1:0",
    ] {
        let p = lookup(id).unwrap_or_else(|| panic!("{id} should resolve"));
        assert!((p.input_per_million - 10.0).abs() < f64::EPSILON, "{id}");
        assert!((p.output_per_million - 50.0).abs() < f64::EPSILON, "{id}");
    }
}

#[test]
fn lookup_sonnet_tier() {
    for id in [
        "us.anthropic.claude-sonnet-4-5-20251015-v1:0",
        "us.anthropic.claude-sonnet-4-6-v1",
        "us.anthropic.claude-sonnet-5-v1",
    ] {
        let p = lookup(id).unwrap_or_else(|| panic!("{id} should resolve"));
        assert!((p.input_per_million - 3.0).abs() < f64::EPSILON, "{id}");
        assert!((p.output_per_million - 15.0).abs() < f64::EPSILON, "{id}");
    }
}

#[test]
fn lookup_haiku_4_5() {
    let p =
        lookup("us.anthropic.claude-haiku-4-5-20251001-v1:0").expect("Haiku 4.5 should resolve");
    assert!((p.input_per_million - 1.0).abs() < f64::EPSILON);
    assert!((p.output_per_million - 5.0).abs() < f64::EPSILON);
}

/// Opus 4 and Opus 4.5 must price differently — confirms the legacy
/// special case and the tier fallback stay apart.
#[test]
fn opus_4_and_4_5_have_distinct_pricing() {
    let p4 = lookup("us.anthropic.claude-opus-4-20250514-v1:0").unwrap();
    let p45 = lookup("us.anthropic.claude-opus-4-5-20251101-v1:0").unwrap();
    assert!(
        (p4.input_per_million - p45.input_per_million).abs() > 1.0,
        "Opus 4 ($15) and Opus 4.5 ($5) must price differently"
    );
}
