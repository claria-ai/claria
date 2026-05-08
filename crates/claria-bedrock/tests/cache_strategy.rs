//! Tests for `CacheStrategy::cache_system_prefix` — the gate that decides
//! whether to attach a Bedrock `cachePoint` block to the system prefix.

use claria_bedrock::chat::CacheStrategy;

#[test]
fn disabled_strategy_never_caches() {
    let s = CacheStrategy::disabled();
    assert!(!s.cache_system_prefix(&"x".repeat(10_000)));
}

#[test]
fn enabled_but_unsupported_model_does_not_cache() {
    let mut s = CacheStrategy::enabled();
    s.model_supports_caching = false;
    assert!(!s.cache_system_prefix(&"x".repeat(10_000)));
}

#[test]
fn enabled_supported_short_prefix_skipped() {
    let mut s = CacheStrategy::enabled();
    s.model_supports_caching = true;
    // Below the 1,200 token / ~4,800 char floor.
    let short = "x".repeat(2_000);
    assert!(!s.cache_system_prefix(&short));
}

#[test]
fn enabled_supported_long_prefix_emits_cache_point() {
    let mut s = CacheStrategy::enabled();
    s.model_supports_caching = true;
    // 6,000 chars ÷ 4 ≈ 1500 tokens — above the floor.
    let long = "x".repeat(6_000);
    assert!(s.cache_system_prefix(&long));
}

#[test]
fn floor_is_configurable() {
    let s = CacheStrategy {
        enabled: true,
        min_prefix_tokens: 100,
        model_supports_caching: true,
    };
    let prefix = "x".repeat(500); // ~125 tokens
    assert!(s.cache_system_prefix(&prefix));
}
