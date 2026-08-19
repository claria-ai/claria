//! Tests for `CacheStrategy` — the gates that decide whether to attach
//! Bedrock `cachePoint` blocks to the system prefix and conversation tail.

use claria_bedrock::chat::{CacheStrategy, ChatMessage, ChatRole};
use claria_core::model_id::{CacheTtlChoice, ModelCapabilities};

/// A caching family on the common 1,024-token floor, so the gate sits at
/// 1,208 estimated tokens (~4,832 characters).
const CACHING_MODEL: &str = "us.anthropic.claude-sonnet-4-6-v1";
/// A family with no prompt caching at all.
const NON_CACHING_MODEL: &str = "us.amazon.nova-pro-v1:0";
/// The primary user's model, whose real floor is four times the common one.
const HIGH_FLOOR_MODEL: &str = "us.anthropic.claude-opus-4-6-v1";

fn strategy(model_id: &str) -> CacheStrategy {
    CacheStrategy::enabled_for_model(ModelCapabilities::for_id(model_id), CacheTtlChoice::OneHour)
}

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
    let s = strategy(NON_CACHING_MODEL);
    assert!(!s.cache_system_prefix(&"x".repeat(10_000)));
    assert!(!s.cache_conversation_tail(&"x".repeat(10_000), &[message(ChatRole::User, 10_000)]));
}

#[test]
fn enabled_supported_short_prefix_skipped() {
    let s = strategy(CACHING_MODEL);
    // Below the 1,208 token / ~4,832 char floor.
    let short = "x".repeat(2_000);
    assert!(!s.cache_system_prefix(&short));
}

#[test]
fn enabled_supported_long_prefix_emits_cache_point() {
    let s = strategy(CACHING_MODEL);
    // 6,000 chars ÷ 4 ≈ 1500 tokens — above the floor.
    let long = "x".repeat(6_000);
    assert!(s.cache_system_prefix(&long));
}

/// The floor is the model's, not a constant. A prefix that caches on a
/// 1,024-token family is a silent no-op on Opus 4.6, which needs 4,096 —
/// so the gate must refuse to mark it there and mark it once it clears.
#[test]
fn the_floor_follows_the_model_not_a_fixed_constant() {
    let common = strategy(CACHING_MODEL);
    let high = strategy(HIGH_FLOOR_MODEL);
    // ~1,500 estimated tokens: over the common floor, far under Opus 4.6's.
    let middling = "x".repeat(6_000);
    assert!(common.cache_system_prefix(&middling));
    assert!(!high.cache_system_prefix(&middling));
    // ~5,000 estimated tokens clears 4,096 plus its slack.
    assert!(high.cache_system_prefix(&"x".repeat(20_000)));
}

#[test]
fn conversation_tail_counts_system_and_messages_against_the_floor() {
    let s = strategy(CACHING_MODEL);
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
