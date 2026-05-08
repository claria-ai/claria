//! Tests for `claria_bedrock::tokens` — verifies extraction of cache fields
//! and round-trip of `TurnUsage`.

use aws_sdk_bedrockruntime::types::TokenUsage as SdkTokenUsage;
use claria_bedrock::tokens::{empty_turn_usage, extract_turn_usage};

fn build_usage(
    input: i32,
    output: i32,
    cache_read: Option<i32>,
    cache_write: Option<i32>,
) -> SdkTokenUsage {
    let mut b = SdkTokenUsage::builder()
        .input_tokens(input)
        .output_tokens(output);
    if let Some(r) = cache_read {
        b = b.cache_read_input_tokens(r);
    }
    if let Some(w) = cache_write {
        b = b.cache_write_input_tokens(w);
    }
    b.total_tokens(input + output)
        .build()
        .expect("token usage builds")
}

#[test]
fn extract_turn_usage_no_cache_active() {
    let sdk_usage = build_usage(1000, 200, None, None);
    let usage = extract_turn_usage(&sdk_usage, "us.anthropic.claude-sonnet-4-20250514-v1:0");

    assert_eq!(usage.input_tokens, 1000);
    assert_eq!(usage.output_tokens, 200);
    assert_eq!(usage.cache_read_input_tokens, 0);
    assert_eq!(usage.cache_write_input_tokens, 0);
    assert!(
        usage.cost_usd > 0.0,
        "cost should be non-zero with valid pricing"
    );
    assert!(usage.pricing_version >= 1);
    assert_eq!(usage.model_id, "us.anthropic.claude-sonnet-4-20250514-v1:0");
}

#[test]
fn extract_turn_usage_with_cache_active() {
    let sdk_usage = build_usage(100, 50, Some(900), Some(0));
    let usage = extract_turn_usage(&sdk_usage, "us.anthropic.claude-sonnet-4-20250514-v1:0");

    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.cache_read_input_tokens, 900);
    assert_eq!(usage.cache_write_input_tokens, 0);
    // Phase 1 leaves cache prices at 0; cost only reflects the un-cached
    // input + output.
    assert!(usage.cost_usd >= 0.0);
}

#[test]
fn extract_turn_usage_unknown_model_zero_cost_zero_version() {
    let sdk_usage = build_usage(1000, 200, None, None);
    let usage = extract_turn_usage(&sdk_usage, "us.amazon.titan-text-v1:0");

    assert_eq!(usage.cost_usd, 0.0);
    assert_eq!(usage.pricing_version, 0);
    assert_eq!(usage.input_tokens, 1000);
}

#[test]
fn empty_turn_usage_keeps_model_id() {
    let usage = empty_turn_usage("us.anthropic.claude-opus-4-20250514-v1:0");
    assert_eq!(usage.input_tokens, 0);
    assert_eq!(usage.output_tokens, 0);
    assert_eq!(usage.cache_read_input_tokens, 0);
    assert_eq!(usage.cache_write_input_tokens, 0);
    assert_eq!(usage.cost_usd, 0.0);
    assert_eq!(usage.pricing_version, 0);
    assert_eq!(usage.model_id, "us.anthropic.claude-opus-4-20250514-v1:0");
}

#[test]
fn turn_usage_total_input_tokens_sums_all_three() {
    let sdk_usage = build_usage(100, 50, Some(900), Some(50));
    let usage = extract_turn_usage(&sdk_usage, "us.anthropic.claude-sonnet-4-20250514-v1:0");

    assert_eq!(usage.total_input_tokens(), 1050);
}

#[test]
fn turn_usage_serde_round_trips() {
    let sdk_usage = build_usage(1000, 200, Some(500), Some(50));
    let usage = extract_turn_usage(&sdk_usage, "us.anthropic.claude-sonnet-4-20250514-v1:0");

    let json = serde_json::to_string(&usage).expect("serialise");
    let back: claria_core::models::turn_usage::TurnUsage =
        serde_json::from_str(&json).expect("deserialise");

    assert_eq!(back.input_tokens, usage.input_tokens);
    assert_eq!(back.cache_read_input_tokens, usage.cache_read_input_tokens);
    assert_eq!(back.cost_usd, usage.cost_usd);
    assert_eq!(back.model_id, usage.model_id);
}
