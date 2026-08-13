//! Single owner of Bedrock model-ID grammar and Claude model capabilities.
//!
//! Knowledge about how Bedrock model identifiers are shaped (inference-profile
//! scope prefixes, release-date stamps) and which Claude families support
//! which features lives here and nowhere else. Callers must never write their
//! own `contains("claude-...")` checks — they ask this table.
//!
//! Capabilities are expressed as **denylists of known-lacking families**, so
//! a newly shipped Claude generation gets modern behavior by default instead
//! of silently losing features until someone edits an allowlist.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Time-to-live for a Bedrock prompt-cache entry.
///
/// Lives next to the capability table because which TTLs a model accepts is
/// model knowledge: `FiveMinutes` is the server default every caching model
/// supports, while `OneHour` requires
/// [`ModelCapabilities::supports_extended_cache_ttl`]. Persisted on
/// `TurnUsage` so historical costs price cache writes at the rate that was
/// actually in effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CacheTtlChoice {
    FiveMinutes,
    OneHour,
}

impl CacheTtlChoice {
    /// Stable snake_case name, matching the serde representation — for log
    /// fields and audit details.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FiveMinutes => "five_minutes",
            Self::OneHour => "one_hour",
        }
    }
}

/// Strip a cross-region inference-profile scope prefix (`us.`, `eu.`,
/// `apac.`, `global.`) from a model ID, returning the bare foundation model
/// ID. IDs without a known scope prefix are returned unchanged.
///
/// The allowlist is exact: provider prefixes such as `anthropic.` are never
/// treated as scopes.
pub fn strip_scope_prefix(model_id: &str) -> &str {
    let Some((scope, foundation_model_id)) = model_id.split_once('.') else {
        return model_id;
    };
    if matches!(scope, "us" | "eu" | "apac" | "global") {
        foundation_model_id
    } else {
        model_id
    }
}

/// The 8-digit release date stamp embedded in a foundation model ID
/// (`anthropic.claude-haiku-4-5-20251001-v1:0` → 20251001), or 0 if absent.
///
/// Orders correctly across both naming shapes
/// (`claude-3-5-haiku-20241022`, `claude-haiku-4-5-20251001`).
pub fn release_date_stamp(model_id: &str) -> u32 {
    let bytes = model_id.as_bytes();
    let mut run = 0;
    // Iterate one past the end so a trailing digit run is still checked.
    for i in 0..=bytes.len() {
        if i < bytes.len() && bytes[i].is_ascii_digit() {
            run += 1;
        } else {
            // Exactly 8 digits is a date stamp; shorter runs are version numbers.
            if run == 8 {
                return model_id[i - 8..i].parse().unwrap_or(0);
            }
            run = 0;
        }
    }
    0
}

/// Choose the model to run Bedrock `CountTokens` against from a set of
/// available foundation model IDs: the newest available Haiku by release
/// date stamp.
///
/// Not every model supports `CountTokens` (e.g. Fable rejects it with a
/// `ValidationException`), so counting never uses the conversation model.
/// Haiku has reliably supported the API across generations.
pub fn preferred_counting_model<I, S>(model_ids: I) -> Option<S>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    model_ids
        .into_iter()
        .filter(|id| id.as_ref().contains("haiku"))
        .max_by_key(|id| release_date_stamp(id.as_ref()))
}

/// The Claude family stem a model ID belongs to, or `None` for IDs outside
/// the families this table knows.
///
/// Tests assert every supported model resolves to a known family, so a new
/// Claude generation that introduces a new stem fails loudly and forces a
/// capability-table review instead of silently taking defaults.
pub fn known_claude_family(model_id: &str) -> Option<&'static str> {
    let id = model_id.to_ascii_lowercase();
    if !id.contains("anthropic.claude") {
        return None;
    }
    const FAMILIES: &[&str] = &[
        "claude-fable",
        "claude-mythos",
        "claude-opus",
        "claude-sonnet",
        "claude-haiku",
        "claude-3-5-haiku",
        "claude-3-5-sonnet",
        "claude-3-7-sonnet",
        "claude-3-opus",
        "claude-3-sonnet",
        "claude-3-haiku",
        "claude-v2",
        "claude-instant",
    ];
    // Longest match wins so `claude-3-5-haiku` is not reported as the
    // `claude-haiku` tier.
    FAMILIES
        .iter()
        .filter(|family| id.contains(*family))
        .max_by_key(|family| family.len())
        .copied()
}

/// What a given Bedrock model ID (inference profile or bare foundation ID)
/// is known to support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    /// Whether the family supports the Converse tool-use protocol used by
    /// the report writer. Denylist: `claude-v2` and `claude-instant`.
    pub report_tools: bool,
    /// Conservative context window for the model/profile. Explicit context
    /// variants (`:48k`, `:200k`, `:1m`) take precedence; Claude 3/4/5
    /// profiles otherwise have a 200k-token window. Unknown providers get a
    /// safe 32k ceiling.
    pub context_window_tokens: u32,
    /// Whether the family honours Bedrock prompt-caching `cachePoint`
    /// blocks. Denylist of known-non-caching families, so claude-*-5 and
    /// newer generations get caching by default.
    pub prompt_caching: bool,
    /// Whether the family accepts the extended 1-hour cache TTL
    /// (`cachePoint.ttl = "1h"`). Denylist: everything on the caching
    /// denylist, every `claude-3-*` family, and the dated 4.0/4.1
    /// generation — AWS gates the 1-hour TTL to Claude 4.5 and newer, so
    /// new generations get it by default.
    pub supports_extended_cache_ttl: bool,
    /// Whether the family accepts adaptive thinking
    /// (`additionalModelRequestFields.thinking = {type: "adaptive"}`).
    /// Denylist: everything predating the extended cache TTL plus the 4.5
    /// minor generation — adaptive thinking arrived with Claude 4.6, so
    /// newer generations get it by default.
    pub adaptive_thinking: bool,
    /// Whether the family accepts `output_config.effort` via
    /// `additionalModelRequestFields`. Effort shipped with Opus 4.5 and
    /// became tier-wide with 4.6 — Sonnet 4.5 and Haiku 4.5 reject it, so
    /// the denylist mirrors adaptive thinking's with an Opus 4.5 carve-out.
    pub effort_parameter: bool,
    /// Whether non-default `temperature`/`top_p` are accepted. Claude
    /// 4.7-class and newer models REJECT them, so this is an allowlist of
    /// the generations that still take sampling parameters (through 4.6) —
    /// unknown and future families deliberately resolve to false, matching
    /// the modern no-sampling contract.
    pub sampling_params: bool,
}

impl ModelCapabilities {
    pub fn for_id(model_id: &str) -> Self {
        let id = model_id.to_ascii_lowercase();
        let is_claude = id.contains("anthropic.claude");
        let legacy_pre_tools = id.contains("claude-v2") || id.contains("claude-instant");

        let context_window_tokens = if id.contains(":48k") {
            48_000
        } else if id.contains(":200k") {
            200_000
        } else if id.contains(":1m") || id.contains(":1000k") {
            1_000_000
        } else if is_claude {
            200_000
        } else {
            32_000
        };

        // Families that predate Bedrock prompt caching. Claude 3.5 Haiku and
        // 3.7 Sonnet onward support it; 3.5 Sonnet never left preview.
        let no_caching = legacy_pre_tools
            || id.contains("claude-3-opus")
            || id.contains("claude-3-sonnet")
            || id.contains("claude-3-haiku")
            || id.contains("claude-3-5-sonnet");

        // Families that cache but predate the extended 1-hour TTL: every
        // claude-3-x family plus the dated 4.0/4.1 generation (Opus 4,
        // Opus 4.1, Sonnet 4). Minor-versioned 4.5+ IDs fall through to
        // modern-default true.
        let no_extended_ttl = no_caching
            || id.contains("claude-3-")
            || family_with_date_stamp(&id, "claude-opus-4")
            || family_with_date_stamp(&id, "claude-opus-4-1")
            || family_with_date_stamp(&id, "claude-sonnet-4");

        // Adaptive thinking arrived with Claude 4.6: exclude everything
        // pre-4.5 (the extended-TTL denylist) plus the 4.5 minor generation.
        let no_adaptive_thinking = no_extended_ttl
            || id.contains("claude-opus-4-5")
            || id.contains("claude-sonnet-4-5")
            || id.contains("claude-haiku-4-5");

        // Effort: Opus 4.5 introduced it; Sonnet 4.5 and Haiku 4.5 reject
        // it; every 4.6+ generation accepts it.
        let no_effort = no_adaptive_thinking && !id.contains("claude-opus-4-5");

        // Generations through 4.6 accept sampling parameters; 4.7-class and
        // newer reject them, so this list enumerates the accepting past and
        // future families default to false.
        let accepts_sampling = id.contains("claude-3-")
            || legacy_pre_tools
            || family_with_date_stamp(&id, "claude-opus-4")
            || family_with_date_stamp(&id, "claude-opus-4-1")
            || family_with_date_stamp(&id, "claude-sonnet-4")
            || id.contains("claude-opus-4-5")
            || id.contains("claude-opus-4-6")
            || id.contains("claude-sonnet-4-5")
            || id.contains("claude-sonnet-4-6")
            || id.contains("claude-haiku-4-5")
            || id.contains("claude-haiku-4-6");

        Self {
            report_tools: is_claude && !legacy_pre_tools,
            context_window_tokens,
            prompt_caching: is_claude && !no_caching,
            supports_extended_cache_ttl: is_claude && !no_extended_ttl,
            adaptive_thinking: is_claude && !no_adaptive_thinking,
            effort_parameter: is_claude && !no_effort,
            sampling_params: is_claude && accepts_sampling,
        }
    }
}

/// True when `id` contains `family` followed by `-` and a release-date
/// stamp (4+ digits), i.e. the dated generation shape
/// (`claude-opus-4-20250514`). A short digit run after the dash is a minor
/// version bump (`claude-sonnet-4-5`), not this generation.
fn family_with_date_stamp(id: &str, family: &str) -> bool {
    id.match_indices(family).any(|(start, _)| {
        let leading_digits = id[start + family.len()..]
            .strip_prefix('-')
            .map(|rest| rest.chars().take_while(char::is_ascii_digit).count())
            .unwrap_or(0);
        leading_digits >= 4
    })
}
