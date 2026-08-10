//! Tests for `CacheStrategy` — the gates that decide whether to attach
//! Bedrock `cachePoint` blocks to the system prefix and conversation tail.

use claria_bedrock::chat::{CacheStrategy, ChatMessage, ChatRole};
use claria_core::model_id::CacheTtlChoice;

fn message(role: ChatRole, chars: usize) -> ChatMessage {
    ChatMessage {
        role,
        content: "x".repeat(chars),
    }
}

#[test]
fn disabled_strategy_never_caches() {
    let s = CacheStrategy::disabled();
    assert!(!s.cache_system_prefix(&"x".repeat(10_000)));
    assert!(!s.cache_conversation_tail(&"x".repeat(10_000), &[message(ChatRole::User, 10_000)]));
}

#[test]
fn enabled_but_unsupported_model_does_not_cache() {
    let s = CacheStrategy::enabled_for_model(false, CacheTtlChoice::OneHour);
    assert!(!s.cache_system_prefix(&"x".repeat(10_000)));
    assert!(!s.cache_conversation_tail(&"x".repeat(10_000), &[message(ChatRole::User, 10_000)]));
}

#[test]
fn enabled_supported_short_prefix_skipped() {
    let s = CacheStrategy::enabled_for_model(true, CacheTtlChoice::OneHour);
    // Below the 1,200 token / ~4,800 char floor.
    let short = "x".repeat(2_000);
    assert!(!s.cache_system_prefix(&short));
}

#[test]
fn enabled_supported_long_prefix_emits_cache_point() {
    let s = CacheStrategy::enabled_for_model(true, CacheTtlChoice::OneHour);
    // 6,000 chars ÷ 4 ≈ 1500 tokens — above the floor.
    let long = "x".repeat(6_000);
    assert!(s.cache_system_prefix(&long));
}

#[test]
fn conversation_tail_counts_system_and_messages_against_the_floor() {
    let s = CacheStrategy::enabled_for_model(true, CacheTtlChoice::OneHour);
    // System alone is under the floor, but system + conversation clears it:
    // the tail marker caches everything ahead of it.
    let short_system = "x".repeat(2_000);
    assert!(!s.cache_conversation_tail(&short_system, &[message(ChatRole::User, 100)]));
    assert!(s.cache_conversation_tail(
        &short_system,
        &[
            message(ChatRole::User, 2_000),
            message(ChatRole::Assistant, 2_000),
        ]
    ));
}
