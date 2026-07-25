use serde::{Deserialize, Serialize};
use specta::Type;

use super::token_count::TokenCount;

/// Pricing per million tokens for a Bedrock model.
///
/// Cache fields cover Anthropic-on-Bedrock prompt caching:
/// - `cache_read_per_million` — typically ~10% of `input_per_million`.
/// - `cache_write_per_million` — typically ~125% of `input_per_million`
///   for the 5-minute TTL tier.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_read_per_million: f64,
    pub cache_write_per_million: f64,
}

impl ModelPricing {
    /// Estimate cost for non-cached token usage. Cache-aware cost lives
    /// in the per-turn capture path; this helper is kept for the legacy
    /// transaction surface.
    pub fn estimate_cost(&self, tokens: TokenCount) -> f64 {
        let input_cost = (tokens.input as f64 / 1_000_000.0) * self.input_per_million;
        let output_cost = (tokens.output as f64 / 1_000_000.0) * self.output_per_million;
        input_cost + output_cost
    }

    /// Cache-aware cost computation. `cache_read` and `cache_write` are
    /// counts of input tokens billed against the cache tiers; `tokens.input`
    /// is the residual (non-cached) input. Output tokens bill at the
    /// standard output rate.
    pub fn estimate_cost_with_cache(
        &self,
        tokens: TokenCount,
        cache_read: u64,
        cache_write: u64,
    ) -> f64 {
        let m = 1_000_000.0;
        (tokens.input as f64 / m) * self.input_per_million
            + (tokens.output as f64 / m) * self.output_per_million
            + (cache_read as f64 / m) * self.cache_read_per_million
            + (cache_write as f64 / m) * self.cache_write_per_million
    }
}

/// A cost estimate shown to the user before a Bedrock call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEstimate {
    pub model_id: String,
    pub estimated_tokens: TokenCount,
    pub estimated_cost_usd: f64,
}
