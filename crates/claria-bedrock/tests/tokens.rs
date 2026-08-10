//! Tests for `claria_bedrock::tokens` — verifies extraction of cache fields
//! and round-trip of `TurnUsage`.

use aws_sdk_bedrockruntime::types::TokenUsage as SdkTokenUsage;
use claria_bedrock::tokens::extract_turn_usage;
use claria_core::model_id::CacheTtlChoice;

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
    let usage = extract_turn_usage(&sdk_usage, "us.anthropic.claude-sonnet-4-20250514-v1:0", None);

    assert_eq!(usage.input_tokens, 1000);
    assert_eq!(usage.output_tokens, 200);
    assert_eq!(usage.cache_read_input_tokens, 0);
    assert_eq!(usage.cache_write_input_tokens, 0);
    assert_eq!(usage.cache_ttl, None);
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
    let usage = extract_turn_usage(&sdk_usage, "us.anthropic.claude-sonnet-4-20250514-v1:0", None);

    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.cache_read_input_tokens, 900);
    assert_eq!(usage.cache_write_input_tokens, 0);
    // Phase 1 leaves cache prices at 0; cost only reflects the un-cached
    // input + output.
    assert!(usage.cost_usd >= 0.0);
}

#[test]
fn extract_turn_usage_records_the_ttl_and_prices_1h_writes_at_their_rate() {
    let sonnet = "us.anthropic.claude-sonnet-4-6-v1";
    let sdk_usage = build_usage(0, 0, Some(0), Some(1_000_000));

    let five_min = extract_turn_usage(&sdk_usage, sonnet, Some(CacheTtlChoice::FiveMinutes));
    let one_hour = extract_turn_usage(&sdk_usage, sonnet, Some(CacheTtlChoice::OneHour));

    assert_eq!(five_min.cache_ttl, Some(CacheTtlChoice::FiveMinutes));
    assert_eq!(one_hour.cache_ttl, Some(CacheTtlChoice::OneHour));
    // Sonnet tier: $3 input → 5-minute writes at 1.25× ($3.75), 1-hour
    // writes at 2× ($6.00) per million.
    assert!((five_min.cost_usd - 3.75).abs() < 1e-9);
    assert!((one_hour.cost_usd - 6.0).abs() < 1e-9);
}

#[test]
fn extract_turn_usage_unknown_model_zero_cost_zero_version() {
    let sdk_usage = build_usage(1000, 200, None, None);
    let usage = extract_turn_usage(&sdk_usage, "us.amazon.titan-text-v1:0", None);

    assert_eq!(usage.cost_usd, 0.0);
    assert_eq!(usage.pricing_version, 0);
    assert_eq!(usage.input_tokens, 1000);
}

#[test]
fn turn_usage_total_input_tokens_sums_all_three() {
    let sdk_usage = build_usage(100, 50, Some(900), Some(50));
    let usage = extract_turn_usage(&sdk_usage, "us.anthropic.claude-sonnet-4-20250514-v1:0", None);

    assert_eq!(usage.total_input_tokens(), 1050);
}

#[test]
fn turn_usage_serde_round_trips() {
    let sdk_usage = build_usage(1000, 200, Some(500), Some(50));
    let usage = extract_turn_usage(
        &sdk_usage,
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        Some(CacheTtlChoice::OneHour),
    );

    let json = serde_json::to_string(&usage).expect("serialise");
    let back: claria_core::models::turn_usage::TurnUsage =
        serde_json::from_str(&json).expect("deserialise");

    assert_eq!(back.input_tokens, usage.input_tokens);
    assert_eq!(back.cache_read_input_tokens, usage.cache_read_input_tokens);
    assert_eq!(back.cache_ttl, Some(CacheTtlChoice::OneHour));
    assert_eq!(back.cost_usd, usage.cost_usd);
    assert_eq!(back.model_id, usage.model_id);
}

#[test]
fn turn_usage_without_a_cache_ttl_field_deserializes_as_none() {
    // Chat history persisted before the TTL field existed.
    let legacy = serde_json::json!({
        "model_id": "us.anthropic.claude-sonnet-4-20250514-v1:0",
        "input_tokens": 10,
        "output_tokens": 5,
        "cache_read_input_tokens": 0,
        "cache_write_input_tokens": 0,
        "cost_usd": 0.0001,
        "pricing_version": 4
    });
    let usage: claria_core::models::turn_usage::TurnUsage =
        serde_json::from_value(legacy).expect("legacy usage deserializes");
    assert_eq!(usage.cache_ttl, None);
}
