use serde::{Deserialize, Serialize};
use specta::Type;

use super::token_count::TokenCount;
use crate::model_id::CacheTtlChoice;

/// Pricing per million tokens for a Bedrock model.
///
/// Cache fields cover Anthropic-on-Bedrock prompt caching:
/// - `cache_read_per_million` — typically ~10% of `input_per_million`.
/// - `cache_write_per_million` — typically ~125% of `input_per_million`
///   for the 5-minute TTL tier.
/// - `cache_write_1h_per_million` — typically ~200% of `input_per_million`
///   for the extended 1-hour TTL tier.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_read_per_million: f64,
    pub cache_write_per_million: f64,
    /// `#[serde(default)]` so pricing serialized before the 1-hour tier
    /// existed still deserializes; an absent (0.0) rate falls back to the
    /// 5-minute write rate rather than pricing 1-hour writes at zero.
    #[serde(default)]
    pub cache_write_1h_per_million: f64,
}

impl ModelPricing {
    /// The cache-write rate for the TTL actually used on a turn. `None`
    /// means the 5-minute/legacy default; a missing 1-hour rate (0.0, from
    /// pre-1h serialized pricing) also falls back to the 5-minute rate so
    /// writes are never priced at zero.
    pub fn cache_write_rate_per_million(&self, cache_ttl: Option<CacheTtlChoice>) -> f64 {
        match cache_ttl {
            Some(CacheTtlChoice::OneHour) if self.cache_write_1h_per_million > 0.0 => {
                self.cache_write_1h_per_million
            }
            _ => self.cache_write_per_million,
        }
    }

    /// Cache-aware cost computation. `cache_read` and `cache_write` are
    /// counts of input tokens billed against the cache tiers; `tokens.input`
    /// is the residual (non-cached) input. Output tokens bill at the
    /// standard output rate, and cache writes at the rate of the TTL used.
    pub fn estimate_cost_with_cache(
        &self,
        tokens: TokenCount,
        cache_read: u64,
        cache_write: u64,
        cache_ttl: Option<CacheTtlChoice>,
    ) -> f64 {
        let m = 1_000_000.0;
        (tokens.input as f64 / m) * self.input_per_million
            + (tokens.output as f64 / m) * self.output_per_million
            + (cache_read as f64 / m) * self.cache_read_per_million
            + (cache_write as f64 / m) * self.cache_write_rate_per_million(cache_ttl)
    }
}

/// A cost estimate shown to the user before a Bedrock call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEstimate {
    pub model_id: String,
    pub estimated_tokens: TokenCount,
    pub estimated_cost_usd: f64,
}
