use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::token_count::TokenCount;

/// Pricing per million tokens for a Bedrock model.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

impl ModelPricing {
    pub fn estimate_cost(&self, tokens: TokenCount) -> f64 {
        let input_cost = (tokens.input as f64 / 1_000_000.0) * self.input_per_million;
        let output_cost = (tokens.output as f64 / 1_000_000.0) * self.output_per_million;
        input_cost + output_cost
    }
}

/// A cost estimate shown to the user before a Bedrock call.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CostEstimate {
    pub model_id: String,
    pub estimated_tokens: TokenCount,
    pub estimated_cost_usd: f64,
}
