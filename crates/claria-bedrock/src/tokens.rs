use claria_core::models::cost::ModelPricing;
use claria_core::models::token_count::{TokenCount, TokenUsage};

/// Extract token counts from a Bedrock Converse response.
pub fn extract_token_usage(
    usage: &aws_sdk_bedrockruntime::types::TokenUsage,
) -> TokenCount {
    TokenCount {
        input: usage.input_tokens as u64,
        output: usage.output_tokens as u64,
    }
}

/// Calculate the cost for a token count given model pricing.
pub fn calculate_cost(tokens: TokenCount, pricing: &ModelPricing) -> TokenUsage {
    TokenUsage {
        tokens,
        cost_usd: pricing.estimate_cost(tokens),
    }
}

/// Known model pricing (per million tokens).
/// These are approximate and should be updated as pricing changes.
pub fn get_pricing(model_id: &str) -> Option<ModelPricing> {
    match model_id {
        // Claude 4 Opus
        id if id.contains("claude-opus-4") => Some(ModelPricing {
            input_per_million: 15.0,
            output_per_million: 75.0,
        }),
        // Claude 4 Sonnet
        id if id.contains("claude-sonnet-4") => Some(ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        }),
        // Claude 3.5 Haiku
        id if id.contains("claude-haiku") => Some(ModelPricing {
            input_per_million: 0.80,
            output_per_million: 4.0,
        }),
        _ => None,
    }
}
