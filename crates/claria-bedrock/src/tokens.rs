//! Token-usage extraction from Bedrock Converse responses.
//!
//! The chat path uses [`extract_turn_usage`] to build a [`TurnUsage`] from
//! a `&aws_sdk_bedrockruntime::types::TokenUsage` and the model_id used to
//! invoke Bedrock. Pricing is resolved via `claria_billing::pricing::lookup`
//! and stamped with `claria_billing::PRICING_VERSION` so historical totals
//! are reconstructable from chat-history JSON alone.
//!
//! [`calculate_cost`] is retained for the legacy `BedrockTransaction`
//! surface (report generation / anonymization) which still uses the
//! pre-cache `TokenUsage` shape from `claria_core::models::token_count`.

use claria_core::models::{
    cost::ModelPricing,
    token_count::{TokenCount, TokenUsage},
    turn_usage::TurnUsage,
};

/// Extract token counts from a Bedrock Converse response (legacy shape).
///
/// Used by the report-generation / anonymization transaction path; the
/// chat path uses [`extract_turn_usage`].
pub fn extract_token_count(usage: &aws_sdk_bedrockruntime::types::TokenUsage) -> TokenCount {
    TokenCount {
        input: usage.input_tokens as u64,
        output: usage.output_tokens as u64,
    }
}

/// Calculate the cost for a token count given model pricing (legacy shape).
pub fn calculate_cost(tokens: TokenCount, pricing: &ModelPricing) -> TokenUsage {
    TokenUsage {
        tokens,
        cost_usd: pricing.estimate_cost(tokens),
    }
}

/// Build a [`TurnUsage`] from a Bedrock `TokenUsage` block and the model
/// invoked.
///
/// Reads `cache_read_input_tokens` and `cache_write_input_tokens` from the
/// SDK with `unwrap_or(0)` — "no cache active" and "0 cache tokens" are the
/// same fact for billing purposes. Pricing is resolved via
/// `claria_billing::pricing::lookup`; if the model is unknown, `cost_usd`
/// is `0.0` and `pricing_version` is `0`.
pub fn extract_turn_usage(
    usage: &aws_sdk_bedrockruntime::types::TokenUsage,
    model_id: &str,
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
        cost_usd,
        pricing_version,
    }
}

/// Build a `TurnUsage` for a turn whose Converse response carried no
/// `usage` block. Token counts are zero and cost is zero, but the
/// `model_id` is preserved so the UI can still attribute the turn.
pub fn empty_turn_usage(model_id: &str) -> TurnUsage {
    TurnUsage {
        model_id: model_id.to_string(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_input_tokens: 0,
        cache_write_input_tokens: 0,
        cost_usd: 0.0,
        pricing_version: 0,
    }
}
