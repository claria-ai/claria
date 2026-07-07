// Cost / token formatting and accumulation helpers shared across chat
// surfaces. The dollar is the headline; tokens stay in tooltips and
// disclosures.
//
// Rounding rules (per the umbrella issue's plain-English copy table):
//   - cost < $0.01      → "< $0.01"
//   - $0.00            → "$0.00"   (zero is a real number, not missing)
//   - cost < $0.10     → "$0.0XX"  (3 decimals — "about a penny")
//   - cost ≥ $0.10     → "$0.XX"   (2 decimals — currency style)
//   - NaN / unparseable → "cost unavailable"

import type { TurnUsage } from "./tauri";

// ---------------------------------------------------------------------------
// Cost formatting
// ---------------------------------------------------------------------------

export function formatCost(usd: number | null | undefined): string {
  if (usd == null || !Number.isFinite(usd)) return "cost unavailable";
  if (usd === 0) return "$0.00";
  if (usd < 0.01 && usd > 0) return "< $0.01";
  if (usd < 0.10) return `$${usd.toFixed(3)}`;
  return `$${usd.toFixed(2)}`;
}

// ---------------------------------------------------------------------------
// Token formatting
// ---------------------------------------------------------------------------

/**
 * Format a token count for display. Locale-grouped under 10k; k-suffix
 * with one decimal at and above 10k.
 *
 *   123      → "123 tok"
 *   1,234    → "1,234 tok"
 *   12,345   → "12.3k tok"
 *   123,456  → "123.5k tok"
 */
export function formatTokens(n: number | null | undefined): string {
  if (n == null || !Number.isFinite(n)) return "—";
  if (n < 10_000) return `${n.toLocaleString()} tok`;
  return `${(n / 1000).toFixed(1)}k tok`;
}

// ---------------------------------------------------------------------------
// Cache hit rate
// ---------------------------------------------------------------------------

/**
 * Cache hit rate as an integer percentage.
 *
 * `cache_read / (input + cache_read + cache_write)`.
 * Returns 0 when no input tokens were billed.
 */
export function cacheHitPct(usage: TurnUsage | null | undefined): number {
  if (!usage) return 0;
  const total =
    usage.input_tokens + usage.cache_read_input_tokens + usage.cache_write_input_tokens;
  if (total === 0) return 0;
  return Math.round((usage.cache_read_input_tokens / total) * 100);
}

// ---------------------------------------------------------------------------
// Pretty model name (strips vendor + version timestamp)
// ---------------------------------------------------------------------------

/**
 * Map a Bedrock inference profile or foundation model id to a
 * user-facing label.
 *
 *   "us.anthropic.claude-sonnet-4-20250514-v1:0" → "Claude Sonnet 4"
 *   "anthropic.claude-opus-4-20250514-v1:0"     → "Claude Opus 4"
 *   "us.anthropic.claude-3-5-haiku-..."         → "Claude 3.5 Haiku"
 */
export function prettyModel(modelId: string | null | undefined): string {
  if (!modelId) return "Unknown model";
  // Strip scope prefix and provider.
  let stem = modelId.replace(/^[a-z]+\.anthropic\./, "").replace(/^anthropic\./, "");
  // Strip a trailing date/version stamp like "-20250514-v1:0".
  stem = stem.replace(/-\d{6,}-v\d+:\d+$/, "");
  // Map family stems → friendly names.
  if (stem.startsWith("claude-opus-4")) return "Claude Opus 4";
  if (stem.startsWith("claude-sonnet-4")) return "Claude Sonnet 4";
  if (stem.startsWith("claude-3-5-haiku")) return "Claude 3.5 Haiku";
  if (stem.startsWith("claude-haiku")) return "Claude Haiku";
  return stem.replace(/-/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

// ---------------------------------------------------------------------------
// Session usage accumulator
// ---------------------------------------------------------------------------

export interface SessionUsage {
  totalUsd: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheReadTokens: number;
  totalCacheWriteTokens: number;
  /// Estimated dollars saved this session via cache reads vs. the
  /// no-caching counterfactual. Computed locally from the per-turn
  /// `usage.model_id` and the input price for that model — we don't
  /// recompute `cost_usd`, just the savings delta.
  cacheSavedUsd: number;
  turnCount: number;
  /// Turns with no pricing entry (pricing_version 0). When non-zero the
  /// dollar totals are understated, so cost surfaces omit them.
  unknownCostTurns: number;
}

export const EMPTY_SESSION_USAGE: SessionUsage = {
  totalUsd: 0,
  totalInputTokens: 0,
  totalOutputTokens: 0,
  totalCacheReadTokens: 0,
  totalCacheWriteTokens: 0,
  cacheSavedUsd: 0,
  turnCount: 0,
  unknownCostTurns: 0,
};

/**
 * Fold a fresh `TurnUsage` into the running session totals.
 *
 * `null` usages (legacy assistant turns or skipped turns) are no-ops:
 * they don't increment the turn count or cost.
 */
export function accumulateUsage(
  prev: SessionUsage,
  usage: TurnUsage | null | undefined,
  pricingByModel: Map<string, ModelPricingLike> = new Map()
): SessionUsage {
  if (!usage) return prev;
  const pricing = pricingByModel.get(usage.model_id);
  const savedThisTurn = pricing
    ? (usage.cache_read_input_tokens / 1_000_000) *
      (pricing.input_per_million - pricing.cache_read_per_million)
    : 0;
  const costKnown = usage.pricing_version !== 0 && Number.isFinite(usage.cost_usd);
  return {
    totalUsd: prev.totalUsd + (costKnown ? usage.cost_usd : 0),
    totalInputTokens: prev.totalInputTokens + usage.input_tokens,
    totalOutputTokens: prev.totalOutputTokens + usage.output_tokens,
    totalCacheReadTokens: prev.totalCacheReadTokens + usage.cache_read_input_tokens,
    totalCacheWriteTokens: prev.totalCacheWriteTokens + usage.cache_write_input_tokens,
    cacheSavedUsd: prev.cacheSavedUsd + Math.max(0, savedThisTurn),
    turnCount: prev.turnCount + 1,
    unknownCostTurns: prev.unknownCostTurns + (costKnown ? 0 : 1),
  };
}

interface ModelPricingLike {
  input_per_million: number;
  cache_read_per_million: number;
}

// ---------------------------------------------------------------------------
// Pre-flight estimate
// ---------------------------------------------------------------------------

/**
 * Approximate "what will this turn cost?" before sending. The result is
 * deliberately a tilde-prefixed number in the UI — we know the user
 * message length but not the model's response length, so we project a
 * reasonable response size and surface the total.
 *
 * Returns `null` if pricing is unknown — the caller hides the estimate.
 */
export function estimateTurnCost(
  pricing: import("./tauri").ModelPricing | null,
  contextTokens: number,
  pendingMessageChars: number,
  /// Project an assistant response length in tokens. ~200 is a
  /// reasonable conversational default; clamp to the caller's setting.
  projectedOutputTokens: number = 200
): number | null {
  if (!pricing) return null;
  const userTokens = Math.ceil(pendingMessageChars / 4);
  const inputTokens = (contextTokens > 0 ? contextTokens : 0) + userTokens;
  const m = 1_000_000;
  return (
    (inputTokens / m) * pricing.input_per_million +
    (projectedOutputTokens / m) * pricing.output_per_million
  );
}

// ---------------------------------------------------------------------------
// Relative time (used in chat history headers)
// ---------------------------------------------------------------------------

export function humanRelative(iso: string | null | undefined): string {
  if (!iso) return "unknown";
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return "unknown";
  const now = Date.now();
  const diffMs = now - t;
  const seconds = Math.floor(diffMs / 1000);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.floor(hours / 24);
  if (days === 1) return "yesterday";
  if (days < 7) return `${days} days ago`;
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

// ---------------------------------------------------------------------------
// Per-chat-history summary
// ---------------------------------------------------------------------------

export interface ChatHistorySummary {
  /// Total cost in USD; `null` if no turns have usage recorded.
  lifetimeUsd: number | null;
  turnCount: number;
  /// Number of assistant turns that pre-date per-turn usage tracking.
  legacyTurnCount: number;
  /// ISO-8601 timestamp of the most recent turn, or `null`.
  lastActivityIso: string | null;
}

/**
 * Walk a chat-history `messages` array and summarise its lifetime cost.
 * Memoise on `messages.length` in the caller — the operation is O(n) but
 * trivially memoizable.
 */
export function summarizeHistory(
  messages: Array<{
    role: string;
    usage?: TurnUsage | null;
  }>,
  lastActivityIso: string | null = null
): ChatHistorySummary {
  let lifetimeUsd = 0;
  let turnCount = 0;
  let legacyTurnCount = 0;
  let anyUsage = false;
  let anyUnknownCost = false;
  for (const m of messages) {
    if (m.role !== "assistant") continue;
    turnCount += 1;
    if (m.usage) {
      anyUsage = true;
      if (m.usage.pricing_version !== 0 && Number.isFinite(m.usage.cost_usd)) {
        lifetimeUsd += m.usage.cost_usd;
      } else {
        anyUnknownCost = true;
      }
    } else {
      legacyTurnCount += 1;
    }
  }
  return {
    // A turn without a pricing entry would understate the total — omit the
    // figure rather than show a wrong one.
    lifetimeUsd: anyUsage && !anyUnknownCost ? lifetimeUsd : null,
    turnCount,
    legacyTurnCount,
    lastActivityIso,
  };
}
