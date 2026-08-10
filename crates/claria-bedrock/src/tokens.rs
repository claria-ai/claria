//! Token-usage extraction from Bedrock Converse responses.
//!
//! [`extract_turn_usage`] builds a [`TurnUsage`] from a
//! `&aws_sdk_bedrockruntime::types::TokenUsage` and the model_id used to
//! invoke Bedrock. Pricing is resolved via `claria_billing::pricing::lookup`
//! and stamped with `claria_billing::PRICING_VERSION` so historical totals
//! are reconstructable from chat-history JSON alone.

use claria_core::{
    model_id::CacheTtlChoice,
    models::{token_count::TokenCount, turn_usage::TurnUsage},
};

/// Build a [`TurnUsage`] from a Bedrock `TokenUsage` block, the model
/// invoked, and the cache TTL the request's cache points carried (`None`
/// for uncached or default-TTL-era turns).
///
/// Reads `cache_read_input_tokens` and `cache_write_input_tokens` from the
/// SDK with `unwrap_or(0)` — "no cache active" and "0 cache tokens" are the
/// same fact for billing purposes. Pricing is resolved via
/// `claria_billing::pricing::lookup`, with cache writes billed at the rate
/// of the TTL used; if the model is unknown, `cost_usd` is `0.0` and
/// `pricing_version` is `0`.
pub fn extract_turn_usage(
    usage: &aws_sdk_bedrockruntime::types::TokenUsage,
    model_id: &str,
    cache_ttl: Option<CacheTtlChoice>,
) -> TurnUsage {
    let input_tokens = usage.input_tokens.max(0) as u64;
    let output_tokens = usage.output_tokens.max(0) as u64;
    let cache_read_input_tokens = usage.cache_read_input_tokens().unwrap_or(0).max(0) as u64;
    let cache_write_input_tokens = usage.cache_write_input_tokens().unwrap_or(0).max(0) as u64;

    let (cost_usd, pricing_version) = match claria_billing::pricing::lookup(model_id) {
        Some(pricing) => {
            let counts = TokenCount {
                input: input_tokens,
                output: output_tokens,
            };
            let cost = pricing.estimate_cost_with_cache(
                counts,
                cache_read_input_tokens,
                cache_write_input_tokens,
                cache_ttl,
            );
            (cost, claria_billing::pricing::PRICING_VERSION)
        }
        None => (0.0, 0),
    };

    TurnUsage {
        model_id: model_id.to_string(),
        input_tokens,
        output_tokens,
        cache_read_input_tokens,
        cache_write_input_tokens,
        cache_ttl,
        cost_usd,
        pricing_version,
    }
}
