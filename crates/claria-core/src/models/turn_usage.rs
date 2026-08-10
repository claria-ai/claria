//! Per-turn token usage captured from a Bedrock Converse response.
//!
//! Persisted on the assistant `ChatHistoryMessage` so historical chat costs
//! are reconstructable from S3 alone. Cost is computed once at call-time
//! against `pricing_version` so historical totals never silently shift if
//! the pricing table is updated later.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::model_id::CacheTtlChoice;

/// Per-turn token usage captured from a Bedrock Converse response.
///
/// Cache fields are populated by the prompt-caching layer; until then they
/// are `0`, which the UI must treat as "no caching active", not "missing
/// data". `String` field means this type cannot be `Copy`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct TurnUsage {
    /// Inference profile ID actually invoked (e.g.
    /// `us.anthropic.claude-sonnet-4-20250514-v1:0`). Stored per-turn — not
    /// inherited from `ChatHistory.model_id` — because the user can switch
    /// models mid-session and we want each turn priced against the model
    /// that produced it.
    pub model_id: String,

    /// Non-cached input tokens billed at full rate.
    pub input_tokens: u64,

    /// Output tokens billed at output rate.
    pub output_tokens: u64,

    /// Cache-hit input tokens (Anthropic-on-Bedrock cache read). Mirrors the
    /// AWS SDK `TokenUsage::cache_read_input_tokens`. `0` when prompt
    /// caching is not active.
    pub cache_read_input_tokens: u64,

    /// Cache-write input tokens — the first turn that creates the cache
    /// entry. Mirrors the AWS SDK `TokenUsage::cache_write_input_tokens`.
    /// `0` when prompt caching is not active.
    pub cache_write_input_tokens: u64,

    /// TTL carried by the request's cache points. `None` means the
    /// 5-minute default or a legacy/uncached turn — either way cache
    /// writes billed at the 5-minute rate, so absence prices correctly.
    #[serde(default)]
    pub cache_ttl: Option<CacheTtlChoice>,

    /// USD cost computed at the moment of the call against `pricing_version`.
    /// Frozen — never recomputed on read. UI may recompute live for
    /// "what-if" estimates but must keep this value as the audit-of-record.
    pub cost_usd: f64,

    /// Version of the pricing table used to compute `cost_usd`. Bumped each
    /// time `claria-billing::pricing` is edited. `0` means "unknown /
    /// pre-versioning".
    pub pricing_version: u32,
}

impl TurnUsage {
    /// Total billable input tokens including cache reads and writes.
    pub fn total_input_tokens(&self) -> u64 {
        self.input_tokens + self.cache_read_input_tokens + self.cache_write_input_tokens
    }
}
