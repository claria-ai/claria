//! Cost arithmetic against the `claria-billing` pricing table.

use claria_core::{model_id::CacheTtlChoice, models::turn_usage::TurnUsage};
use claria_eval::cost;

const SONNET: &str = "us.anthropic.claude-sonnet-4-5-20250929-v1:0";
const OPUS: &str = "us.anthropic.claude-opus-4-6-20260101-v1:0";

fn usage(
    model_id: &str,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cache_ttl: Option<CacheTtlChoice>,
) -> TurnUsage {
    TurnUsage {
        model_id: model_id.to_string(),
        input_tokens: input,
        output_tokens: output,
        cache_read_input_tokens: cache_read,
        cache_write_input_tokens: cache_write,
        cache_ttl,
        // Deliberately wrong: the harness reprices from the tokens rather
        // than trusting a number captured under an older pricing version.
        cost_usd: 999.0,
        pricing_version: 1,
    }
}

/// Sonnet is $3/M in, $15/M out.
#[test]
fn a_plain_call_prices_at_the_tier_rate() {
    let total = cost::total([&usage(SONNET, 1_000_000, 1_000_000, 0, 0, None)]);
    assert!((total.cost_usd - 18.0).abs() < 1e-9, "{}", total.cost_usd);
    assert!(!total.unpriced_calls);
    assert_eq!(total.total_input_tokens(), 1_000_000);
}

/// Cache reads are 0.1x input, five-minute writes 1.25x, one-hour writes 2x.
#[test]
fn cache_tiers_price_at_their_multipliers() {
    let five_minute = cost::total([&usage(SONNET, 0, 0, 1_000_000, 1_000_000, None)]);
    assert!(
        (five_minute.cost_usd - (0.30 + 3.75)).abs() < 1e-9,
        "{}",
        five_minute.cost_usd
    );

    let one_hour = cost::total([&usage(
        SONNET,
        0,
        0,
        1_000_000,
        1_000_000,
        Some(CacheTtlChoice::OneHour),
    )]);
    assert!(
        (one_hour.cost_usd - (0.30 + 6.0)).abs() < 1e-9,
        "{}",
        one_hour.cost_usd
    );
}

/// Every call in a run is priced against the model that produced it, because
/// the planner and the writer are not the same model.
#[test]
fn a_run_sums_per_call_against_each_calls_own_model() {
    let total = cost::total([
        &usage(SONNET, 1_000_000, 0, 0, 0, None),
        &usage(OPUS, 1_000_000, 0, 0, 0, None),
    ]);
    assert!((total.cost_usd - 8.0).abs() < 1e-9, "{}", total.cost_usd);
    assert_eq!(total.total_input_tokens(), 2_000_000);
}

/// A model the table does not know contributes nothing and flags the total as
/// a floor, rather than silently reporting a cheaper run than happened.
#[test]
fn an_unknown_model_makes_the_total_a_floor() {
    let total = cost::total([
        &usage(SONNET, 1_000_000, 0, 0, 0, None),
        &usage("meta.llama-not-in-the-table", 1_000_000, 0, 0, 0, None),
    ]);
    assert!((total.cost_usd - 3.0).abs() < 1e-9, "{}", total.cost_usd);
    assert!(total.unpriced_calls);
    assert_eq!(total.total_input_tokens(), 2_000_000);
}

/// The harness's arithmetic is the table's arithmetic — not a second copy of
/// the multipliers that can drift from it.
#[test]
fn the_harness_agrees_with_the_pricing_table_directly() {
    let sample = usage(SONNET, 12_345, 6_789, 4_321, 1_234, None);
    let pricing = claria_billing::pricing::lookup(SONNET).expect("sonnet is priced");
    let expected = pricing.estimate_cost_with_cache(
        claria_core::models::token_count::TokenCount {
            input: sample.input_tokens,
            output: sample.output_tokens,
        },
        sample.cache_read_input_tokens,
        sample.cache_write_input_tokens,
        sample.cache_ttl,
    );
    let total = cost::total([&sample]);
    assert!((total.cost_usd - expected).abs() < 1e-12);
}

#[test]
fn no_calls_costs_nothing() {
    let total = cost::total([]);
    assert_eq!(total.cost_usd, 0.0);
    assert!(!total.unpriced_calls);
}
