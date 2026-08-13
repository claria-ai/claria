//! Tests for the central model-ID grammar and capability table.
//!
//! The representative-ID list below covers every Claude family Claria has
//! seen in the wild. When a new generation ships, add its ID here and to
//! `known_claude_family` — the test failing on an unknown family is the
//! forcing function that keeps the capability table reviewed.

use claria_core::model_id::{
    ModelCapabilities, known_claude_family, preferred_counting_model, release_date_stamp,
    strip_scope_prefix,
};

/// One representative ID per Claude family/generation currently supported.
const REPRESENTATIVE_MODEL_IDS: &[&str] = &[
    "anthropic.claude-v2:1",
    "anthropic.claude-instant-v1",
    "us.anthropic.claude-3-opus-20240229-v1:0",
    "us.anthropic.claude-3-sonnet-20240229-v1:0",
    "us.anthropic.claude-3-haiku-20240307-v1:0",
    "us.anthropic.claude-3-5-sonnet-20241022-v2:0",
    "us.anthropic.claude-3-5-haiku-20241022-v1:0",
    "us.anthropic.claude-3-7-sonnet-20250219-v1:0",
    "us.anthropic.claude-opus-4-20250514-v1:0",
    "us.anthropic.claude-sonnet-4-20250514-v1:0",
    "us.anthropic.claude-haiku-4-5-20251001-v1:0",
    "us.anthropic.claude-opus-4-6-v1",
    "us.anthropic.claude-sonnet-4-6-v1",
    "global.anthropic.claude-sonnet-5-v1",
    "us.anthropic.claude-fable-5-v1:0",
    "us.anthropic.claude-mythos-5-v1:0",
];

#[test]
fn every_supported_model_maps_to_a_known_family() {
    for id in REPRESENTATIVE_MODEL_IDS {
        assert!(
            known_claude_family(id).is_some(),
            "{id} is not in the model capability table — add its family to \
             claria-core/src/model_id.rs and review its capabilities"
        );
    }
}

#[test]
fn unknown_families_are_not_silently_classified() {
    assert_eq!(known_claude_family("anthropic.claude-newthing-9"), None);
    assert_eq!(known_claude_family("us.amazon.nova-pro-v1:0"), None);
    assert_eq!(known_claude_family("amazon.titan-text-express-v1"), None);
}

#[test]
fn longest_family_stem_wins() {
    assert_eq!(
        known_claude_family("us.anthropic.claude-3-5-haiku-20241022-v1:0"),
        Some("claude-3-5-haiku")
    );
    assert_eq!(
        known_claude_family("us.anthropic.claude-haiku-4-5-20251001-v1:0"),
        Some("claude-haiku")
    );
}

#[test]
fn scope_prefix_stripping_uses_exact_allowlist() {
    assert_eq!(
        strip_scope_prefix("us.anthropic.claude-sonnet-4-6-v1"),
        "anthropic.claude-sonnet-4-6-v1"
    );
    assert_eq!(
        strip_scope_prefix("eu.anthropic.claude-sonnet-4-6-v1"),
        "anthropic.claude-sonnet-4-6-v1"
    );
    assert_eq!(
        strip_scope_prefix("apac.anthropic.claude-sonnet-4-6-v1"),
        "anthropic.claude-sonnet-4-6-v1"
    );
    assert_eq!(
        strip_scope_prefix("global.anthropic.claude-sonnet-4-6-v1"),
        "anthropic.claude-sonnet-4-6-v1"
    );
    // Provider prefixes are not scopes.
    assert_eq!(
        strip_scope_prefix("anthropic.claude-sonnet-4-6-v1"),
        "anthropic.claude-sonnet-4-6-v1"
    );
    // Unknown short prefixes are not scopes either — no heuristic.
    assert_eq!(
        strip_scope_prefix("mx.anthropic.claude"),
        "mx.anthropic.claude"
    );
    assert_eq!(strip_scope_prefix("no-dots"), "no-dots");
}

#[test]
fn report_tools_denylist_excludes_pre_tool_families() {
    assert!(!ModelCapabilities::for_id("anthropic.claude-v2:1").report_tools);
    assert!(!ModelCapabilities::for_id("anthropic.claude-instant-v1").report_tools);
    assert!(!ModelCapabilities::for_id("amazon.titan-text-express-v1").report_tools);
    assert!(ModelCapabilities::for_id("us.anthropic.claude-sonnet-4-6-v1").report_tools);
    assert!(ModelCapabilities::for_id("us.anthropic.claude-fable-5-v1:0").report_tools);
}

#[test]
fn context_window_variants_take_precedence() {
    assert_eq!(
        ModelCapabilities::for_id("us.anthropic.claude:48k").context_window_tokens,
        48_000
    );
    assert_eq!(
        ModelCapabilities::for_id("us.anthropic.claude-sonnet-4-6:200k").context_window_tokens,
        200_000
    );
    assert_eq!(
        ModelCapabilities::for_id("us.anthropic.claude-sonnet-4-6:1m").context_window_tokens,
        1_000_000
    );
    assert_eq!(
        ModelCapabilities::for_id("us.anthropic.claude-sonnet-4-6-v1").context_window_tokens,
        200_000
    );
    assert_eq!(
        ModelCapabilities::for_id("amazon.titan-text-express-v1").context_window_tokens,
        32_000
    );
}

#[test]
fn prompt_caching_is_denied_only_for_known_non_caching_families() {
    // Known non-caching families.
    for id in [
        "anthropic.claude-v2:1",
        "anthropic.claude-instant-v1",
        "us.anthropic.claude-3-opus-20240229-v1:0",
        "us.anthropic.claude-3-sonnet-20240229-v1:0",
        "us.anthropic.claude-3-haiku-20240307-v1:0",
        "us.anthropic.claude-3-5-sonnet-20241022-v2:0",
    ] {
        assert!(!ModelCapabilities::for_id(id).prompt_caching, "{id}");
    }
    // Caching-capable families, including newer generations by default.
    for id in [
        "us.anthropic.claude-3-5-haiku-20241022-v1:0",
        "us.anthropic.claude-3-7-sonnet-20250219-v1:0",
        "us.anthropic.claude-opus-4-20250514-v1:0",
        "us.anthropic.claude-sonnet-4-6-v1",
        "global.anthropic.claude-sonnet-5-v1",
        "us.anthropic.claude-fable-5-v1:0",
    ] {
        assert!(ModelCapabilities::for_id(id).prompt_caching, "{id}");
    }
    // Non-Claude models never get a cache point.
    assert!(!ModelCapabilities::for_id("us.amazon.nova-pro-v1:0").prompt_caching);
}

#[test]
fn extended_cache_ttl_is_denied_only_for_pre_45_families() {
    // Everything on the caching denylist, plus caching-capable claude-3-x
    // families, plus the dated 4.0/4.1 generation — none accept ttl "1h".
    for id in [
        "anthropic.claude-v2:1",
        "anthropic.claude-instant-v1",
        "us.anthropic.claude-3-opus-20240229-v1:0",
        "us.anthropic.claude-3-5-sonnet-20241022-v2:0",
        "us.anthropic.claude-3-5-haiku-20241022-v1:0",
        "us.anthropic.claude-3-7-sonnet-20250219-v1:0",
        "us.anthropic.claude-opus-4-20250514-v1:0",
        "us.anthropic.claude-opus-4-1-20250805-v1:0",
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
    ] {
        assert!(
            !ModelCapabilities::for_id(id).supports_extended_cache_ttl,
            "{id}"
        );
    }
    // The 4.5 generation and everything newer gets the 1-hour TTL by
    // default — including dated 4.5 IDs and undated 4.6+ profile shapes.
    for id in [
        "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
        "us.anthropic.claude-haiku-4-5-20251001-v1:0",
        "us.anthropic.claude-opus-4-6-v1",
        "us.anthropic.claude-sonnet-4-6-v1",
        "global.anthropic.claude-sonnet-5-v1",
        "us.anthropic.claude-fable-5-v1:0",
        "us.anthropic.claude-mythos-5-v1:0",
    ] {
        assert!(
            ModelCapabilities::for_id(id).supports_extended_cache_ttl,
            "{id}"
        );
    }
    // Unknown providers never get the extended TTL.
    assert!(!ModelCapabilities::for_id("us.amazon.nova-pro-v1:0").supports_extended_cache_ttl);
}

#[test]
fn adaptive_thinking_and_effort_follow_their_generation_boundaries() {
    // Pre-4.6 families never get adaptive thinking.
    for id in [
        "anthropic.claude-v2:1",
        "us.anthropic.claude-3-7-sonnet-20250219-v1:0",
        "us.anthropic.claude-opus-4-20250514-v1:0",
        "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
        "us.anthropic.claude-haiku-4-5-20251001-v1:0",
    ] {
        assert!(!ModelCapabilities::for_id(id).adaptive_thinking, "{id}");
    }
    // 4.6 and newer generations get it by default.
    for id in [
        "us.anthropic.claude-opus-4-6-v1",
        "us.anthropic.claude-sonnet-4-6-v1",
        "global.anthropic.claude-sonnet-5-v1",
        "us.anthropic.claude-fable-5-v1:0",
    ] {
        assert!(ModelCapabilities::for_id(id).adaptive_thinking, "{id}");
    }
    // Effort shipped with Opus 4.5 and became tier-wide with 4.6 — Sonnet
    // 4.5 and Haiku 4.5 reject it.
    assert!(!ModelCapabilities::for_id("us.anthropic.claude-opus-4-20250514-v1:0").effort_parameter);
    assert!(ModelCapabilities::for_id("us.anthropic.claude-opus-4-5-20251101-v1:0").effort_parameter);
    assert!(
        !ModelCapabilities::for_id("us.anthropic.claude-sonnet-4-5-20250929-v1:0").effort_parameter
    );
    assert!(
        !ModelCapabilities::for_id("us.anthropic.claude-haiku-4-5-20251001-v1:0").effort_parameter
    );
    assert!(ModelCapabilities::for_id("us.anthropic.claude-opus-4-6-v1").effort_parameter);
    assert!(ModelCapabilities::for_id("us.anthropic.claude-fable-5-v1:0").effort_parameter);
    assert!(!ModelCapabilities::for_id("us.amazon.nova-pro-v1:0").effort_parameter);
}

#[test]
fn sampling_params_are_an_allowlist_that_fails_closed_for_new_generations() {
    // Generations through 4.6 accept temperature/top_p.
    for id in [
        "anthropic.claude-v2:1",
        "us.anthropic.claude-3-haiku-20240307-v1:0",
        "us.anthropic.claude-opus-4-20250514-v1:0",
        "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
        "us.anthropic.claude-haiku-4-5-20251001-v1:0",
        "us.anthropic.claude-opus-4-6-v1",
        "us.anthropic.claude-sonnet-4-6-v1",
    ] {
        assert!(ModelCapabilities::for_id(id).sampling_params, "{id}");
    }
    // 4.7-class and newer reject them — and unknown future families fail
    // closed to the modern no-sampling contract.
    for id in [
        "global.anthropic.claude-sonnet-5-v1",
        "us.anthropic.claude-fable-5-v1:0",
        "us.anthropic.claude-mythos-5-v1:0",
        "us.anthropic.claude-opus-6-v1:0",
        "us.amazon.nova-pro-v1:0",
    ] {
        assert!(!ModelCapabilities::for_id(id).sampling_params, "{id}");
    }
}

#[test]
fn counting_model_picks_newest_haiku_by_release_date() {
    let models = [
        "anthropic.claude-3-5-haiku-20241022-v1:0",
        "anthropic.claude-haiku-4-5-20251001-v1:0",
        "anthropic.claude-sonnet-4-6-v1",
    ];
    assert_eq!(
        preferred_counting_model(models.iter().copied()),
        Some("anthropic.claude-haiku-4-5-20251001-v1:0")
    );
    assert_eq!(
        preferred_counting_model(["anthropic.claude-sonnet-4-6-v1"].iter().copied()),
        None
    );
}

#[test]
fn release_date_stamps_order_across_naming_shapes() {
    assert_eq!(
        release_date_stamp("anthropic.claude-3-5-haiku-20241022-v1:0"),
        20_241_022
    );
    assert_eq!(
        release_date_stamp("anthropic.claude-haiku-4-5-20251001-v1:0"),
        20_251_001
    );
    assert_eq!(release_date_stamp("anthropic.claude-sonnet-4-6-v1"), 0);
}
