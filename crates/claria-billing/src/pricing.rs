//! Bedrock model pricing table and lookup.
//!
//! The pricing *type* (`ModelPricing`) lives in `claria-core` because it is
//! a domain type shared across crates. The pricing *table* lives here
//! because pricing data changes independently of any type definition and
//! `claria-billing` is the natural home for cost-related data.
//!
//! `lookup` strips the inference-profile scope prefix (`us.`, `eu.`,
//! `global.`) and matches on the bare foundation-model family by **exact
//! prefix on the family stem**, killing the `id.contains("claude-opus-4")`
//! substring trap that would silently mis-price a future
//! `claude-opus-4-1`.

use claria_core::models::cost::ModelPricing;

/// Bumped on every edit to the pricing table. Stamped onto
/// `TurnUsage.pricing_version` at capture time so historical totals do not
/// silently shift when prices are updated.
///
/// Phase 1 (capture only, cache fields zero) → version 1.
pub const PRICING_VERSION: u32 = 1;

/// Resolve pricing for a Bedrock model_id.
///
/// Strips an inference-profile scope prefix (`us.`, `eu.`, `global.`) and
/// matches the bare foundation-model family. The family stem is anchored
/// with a dash boundary so `claude-opus-4` does not falsely match
/// `claude-opus-4-1`.
pub fn lookup(model_id: &str) -> Option<ModelPricing> {
    let bare = strip_scope_prefix(model_id);
    // Drop the provider prefix (e.g. `anthropic.`) so we match against the
    // family stem.
    let stem = bare.strip_prefix("anthropic.").unwrap_or(bare);

    // Match the family stem with an explicit dash boundary so a future
    // `claude-opus-4-1` does not match `claude-opus-4`.
    if family_matches(stem, "claude-opus-4") {
        return Some(ModelPricing {
            input_per_million: 15.0,
            output_per_million: 75.0,
            cache_read_per_million: 0.0,
            cache_write_per_million: 0.0,
        });
    }
    if family_matches(stem, "claude-sonnet-4") {
        return Some(ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
            cache_read_per_million: 0.0,
            cache_write_per_million: 0.0,
        });
    }
    if family_matches(stem, "claude-haiku") || family_matches(stem, "claude-3-5-haiku") {
        return Some(ModelPricing {
            input_per_million: 0.80,
            output_per_million: 4.0,
            cache_read_per_million: 0.0,
            cache_write_per_million: 0.0,
        });
    }

    None
}

/// `true` if `stem` equals `family` or starts with `family-` followed by a
/// digit (the version timestamp). This avoids `claude-opus-4` matching
/// `claude-opus-4-1-future`.
fn family_matches(stem: &str, family: &str) -> bool {
    if stem == family {
        return true;
    }
    if let Some(rest) = stem.strip_prefix(family) {
        // The character right after the family stem must be a dash AND the
        // character after that must NOT be another digit (which would
        // indicate a numeric bump like `-1` in `claude-opus-4-1`).
        if let Some(after_dash) = rest.strip_prefix('-') {
            // `after_dash` should start with a digit (e.g. `20250514-v1:0`)
            // — a year-month-day timestamp — and must NOT match a pure
            // numeric version bump like `1-future`.
            //
            // Heuristic: a date stamp starts with `20` followed by 6 more
            // digits before a dash; a numeric bump like `1` is just one or
            // two digits before a dash or end. Require at least 4 leading
            // digits before any non-digit to count as a date stamp.
            let leading_digits = after_dash
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .count();
            return leading_digits >= 4;
        }
    }
    false
}

fn strip_scope_prefix(model_id: &str) -> &str {
    if let Some((prefix, rest)) = model_id.split_once('.') {
        let is_scope = prefix.len() <= 6 && prefix.chars().all(|c| c.is_ascii_lowercase());
        if is_scope && rest.contains('.') {
            return rest;
        }
    }
    model_id
}
