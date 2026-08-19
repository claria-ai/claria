//! What a run cost, priced through `claria-billing`.
//!
//! The pipeline already stamps `cost_usd` onto every `TurnUsage` it captures.
//! This module recomputes from the tokens against the same table so the
//! harness reports one number it derived itself — and so a `TurnUsage` that
//! predates a pricing edit is re-priced rather than trusted.

use claria_core::models::{token_count::TokenCount, turn_usage::TurnUsage};

/// Tokens and dollars for one run, summed over every captured call.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RunCost {
    /// Non-cached input tokens.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
    /// `true` when at least one call named a model the pricing table does not
    /// know, so the total is a floor rather than the price.
    pub unpriced_calls: bool,
}

impl RunCost {
    /// Every input token the account is billed for, cached or not.
    pub fn total_input_tokens(&self) -> u64 {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    /// Fold one call's usage in.
    pub fn add(&mut self, usage: &TurnUsage) {
        self.input_tokens += usage.input_tokens;
        self.output_tokens += usage.output_tokens;
        self.cache_read_tokens += usage.cache_read_input_tokens;
        self.cache_write_tokens += usage.cache_write_input_tokens;
        match claria_billing::pricing::lookup(&usage.model_id) {
            Some(pricing) => {
                self.cost_usd += pricing.estimate_cost_with_cache(
                    TokenCount {
                        input: usage.input_tokens,
                        output: usage.output_tokens,
                    },
                    usage.cache_read_input_tokens,
                    usage.cache_write_input_tokens,
                    usage.cache_ttl,
                );
            }
            None => self.unpriced_calls = true,
        }
    }
}

/// Sum a run's captured usages.
pub fn total<'a>(usages: impl IntoIterator<Item = &'a TurnUsage>) -> RunCost {
    let mut total = RunCost::default();
    for usage in usages {
        total.add(usage);
    }
    total
}
