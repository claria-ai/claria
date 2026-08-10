//! Bedrock model pricing table and lookup.
//!
//! The pricing *type* (`ModelPricing`) lives in `claria-core` because it is
//! a domain type shared across crates. The pricing *table* lives here
//! because pricing data changes independently of any type definition and
//! `claria-billing` is the natural home for cost-related data.
//!
//! `lookup` strips the inference-profile scope prefix (`us.`, `eu.`,
//! `global.`) and matches by **tier** (fable/opus/sonnet/haiku substring),
//! not per-minor-version families. AWS Bedrock charges Anthropic's
//! published per-tier rates, and those rates have been stable within a
//! tier since the 4.5 generation — tier matching keeps costs correct when
//! a new minor version ships without a table edit. The few legacy
//! families whose price differs from their tier's current rate (Opus
//! 4/4.1, Haiku 3.x) are special-cased ahead of the tier match. A model
//! that matches no tier returns `None` and the UI omits cost entirely.

use claria_core::{model_id::strip_scope_prefix, models::cost::ModelPricing};

/// Bumped on every edit to the pricing table. Stamped onto
/// `TurnUsage.pricing_version` at capture time so historical totals do not
/// silently shift when prices are updated.
///
/// Phase 1 (capture only, cache fields zero) → version 1.
/// Phase 2 (prompt caching active) → version 2 with non-zero cache prices.
/// Phase 3 (Claude 4.5+ generation added) → version 3.
/// Phase 4 (tier-level matching; Fable/Mythos and Claude 5 generation) → version 4.
pub const PRICING_VERSION: u32 = 4;

/// Resolve pricing for a Bedrock model_id.
///
/// Strips an inference-profile scope prefix (`us.`, `eu.`, `global.`) and
/// the `anthropic.` provider prefix, then matches legacy families first
/// and tiers second. Returns `None` for models outside the Claude
/// fable/opus/sonnet/haiku tiers.
pub fn lookup(model_id: &str) -> Option<ModelPricing> {
    let bare = strip_scope_prefix(model_id);
    let stem = bare.strip_prefix("anthropic.").unwrap_or(bare);

    // Legacy families priced differently from their tier's current rate.
    // These use the dash-boundary family matcher so e.g. `claude-opus-4`
    // ($15) does not swallow `claude-opus-4-6` ($5).
    if family_matches(stem, "claude-opus-4") || family_matches(stem, "claude-opus-4-1") {
        return Some(tier(15.0, 75.0));
    }
    if family_matches(stem, "claude-3-5-haiku") {
        return Some(tier(0.80, 4.0));
    }
    if family_matches(stem, "claude-3-haiku") {
        return Some(tier(0.25, 1.25));
    }

    // Tier match: current published Anthropic rates per tier.
    if stem.contains("fable") || stem.contains("mythos") {
        return Some(tier(10.0, 50.0));
    }
    if stem.contains("opus") {
        return Some(tier(5.0, 25.0));
    }
    if stem.contains("sonnet") {
        return Some(tier(3.0, 15.0));
    }
    if stem.contains("haiku") {
        return Some(tier(1.0, 5.0));
    }

    None
}

/// Cache pricing follows Anthropic's standard multipliers: reads at 0.1×
/// the input rate, writes at 1.25× (5-minute TTL).
fn tier(input_per_million: f64, output_per_million: f64) -> ModelPricing {
    ModelPricing {
        input_per_million,
        output_per_million,
        cache_read_per_million: input_per_million * 0.10,
        cache_write_per_million: input_per_million * 1.25,
    }
}

/// `true` if `stem` equals `family` or starts with `family-` followed by a
/// date stamp (4+ digits). A short digit run after the dash is a minor
/// version bump (`claude-opus-4-1`), not this family.
fn family_matches(stem: &str, family: &str) -> bool {
    if stem == family {
        return true;
    }
    if let Some(after_dash) = stem
        .strip_prefix(family)
        .and_then(|rest| rest.strip_prefix('-'))
    {
        let leading_digits = after_dash
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .count();
        return leading_digits >= 4;
    }
    false
}
