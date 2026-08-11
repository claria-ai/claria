// Cache-aware cost ledger: a pure presentation-layer derivation over an
// ordered list of turn usages. Explains where each turn's dollars went
// (fresh input, cache write, cache read, output), what the same tokens
// would have cost with caching off, and which turns hit, missed, or warmed
// the cache. Never persisted, never fetched — and never a substitute for
// the frozen per-turn `cost_usd` audit-of-record. All rate selection is
// reused from `cost.ts`.

import { cacheWriteRatePerMillion } from "./cost";
import type { CacheTtlChoice, ModelPricing, TurnUsage } from "./tauri";

const M = 1_000_000;

/** Why a turn read (or didn't read) from the prompt cache. */
export type CacheOutcome =
  /** The first metered turn of the session — nothing to read yet. */
  | { kind: "cold_start" }
  /** The turn read tokens back from cache. */
  | { kind: "hit" }
  /** The prior write is old enough that its TTL is known to have expired. */
  | { kind: "miss"; expiredTtl: CacheTtlChoice }
  /**
   * A recent prior turn wrote cache, but this turn did not read it. The
   * prefix may have changed or the provider may not have reused it; elapsed
   * time does not prove expiry.
   */
  | { kind: "not_reused" }
  /** No reads, and the previous metered turn wrote nothing to miss. */
  | { kind: "no_cache" };

/** A turn's spend split by the rate each token class was billed at. */
export interface TurnCostComponents {
  freshInputUsd: number;
  cacheWriteUsd: number;
  cacheReadUsd: number;
  outputUsd: number;
}

export interface TurnLedgerEntry {
  /** Index into the caller's turn list; unmetered turns keep their slots. */
  index: number;
  usage: TurnUsage;
  /** TTL class of the turn's cache writes (absent TTL = 5-minute default). */
  cacheWriteTtl: CacheTtlChoice;
  outcome: CacheOutcome;
  /** `null` when the model has no entry in the supplied pricing map. */
  components: TurnCostComponents | null;
  /**
   * Sum of the components — recomputed at the supplied rates so bar
   * segments always sum to the bar, independent of the frozen `cost_usd`.
   */
  actualUsd: number | null;
  /**
   * The no-caching counterfactual: every input token (fresh + cache read
   * + cache written) at the full input rate, output at the output rate.
   */
  counterfactualUsd: number | null;
  /**
   * counterfactual − actual, signed. Negative means the turn spent more
   * writing cache than it read back — an investment, not a loss. Clamp at
   * the presentation layer, never here.
   */
  savingsUsd: number | null;
}

export interface CostLedger {
  /** One entry per metered turn, in send order. */
  entries: TurnLedgerEntry[];
  /** Turns with no usage recorded — counted, never treated as zeros. */
  unmeteredTurns: number;
  /**
   * Metered turns whose model has no pricing entry. The dollar totals
   * below exclude them, so they are understated while this is non-zero.
   */
  unpricedTurns: number;
  /** Component totals across priced turns. */
  totals: TurnCostComponents;
  actualUsd: number;
  counterfactualUsd: number;
  /** Signed cumulative savings across priced turns. */
  savingsUsd: number;
  /** `savingsUsd` as a share of the counterfactual, in percent. Signed. */
  savingsPct: number;
  hitCount: number;
  missCount: number;
  notReusedCount: number;
  coldStartCount: number;
}

/**
 * Derive the session ledger from an ordered list of turn usages and the
 * pricing map available at the call site. `null`/`undefined` turns are
 * unmetered: counted separately, never priced as zeros, and skipped when
 * deciding which turn "preceded" another for expiry attribution.
 */
export function buildCostLedger(
  turns: ReadonlyArray<TurnUsage | null | undefined>,
  pricingByModel: ReadonlyMap<string, ModelPricing>,
  completedAt: ReadonlyArray<string | null | undefined> = []
): CostLedger {
  const entries: TurnLedgerEntry[] = [];
  const totals: TurnCostComponents = {
    freshInputUsd: 0,
    cacheWriteUsd: 0,
    cacheReadUsd: 0,
    outputUsd: 0,
  };
  let unmeteredTurns = 0;
  let unpricedTurns = 0;
  let actualUsd = 0;
  let counterfactualUsd = 0;
  let hitCount = 0;
  let missCount = 0;
  let notReusedCount = 0;
  let coldStartCount = 0;
  let prevMetered: TurnUsage | null = null;
  let prevCompletedAt: string | null = null;

  for (let index = 0; index < turns.length; index++) {
    const usage = turns[index];
    if (!usage) {
      unmeteredTurns += 1;
      continue;
    }

    const outcome = classifyOutcome(
      usage,
      prevMetered,
      completedAt[index] ?? null,
      prevCompletedAt
    );
    if (outcome.kind === "hit") hitCount += 1;
    else if (outcome.kind === "miss") missCount += 1;
    else if (outcome.kind === "not_reused") notReusedCount += 1;
    else if (outcome.kind === "cold_start") coldStartCount += 1;

    const pricing = pricingByModel.get(usage.model_id);
    let components: TurnCostComponents | null = null;
    let actual: number | null = null;
    let counterfactual: number | null = null;
    let savings: number | null = null;
    if (pricing) {
      components = {
        freshInputUsd: (usage.input_tokens / M) * pricing.input_per_million,
        cacheWriteUsd:
          (usage.cache_write_input_tokens / M) *
          cacheWriteRatePerMillion(pricing, usage.cache_ttl),
        cacheReadUsd:
          (usage.cache_read_input_tokens / M) * pricing.cache_read_per_million,
        outputUsd: (usage.output_tokens / M) * pricing.output_per_million,
      };
      actual =
        components.freshInputUsd +
        components.cacheWriteUsd +
        components.cacheReadUsd +
        components.outputUsd;
      counterfactual =
        ((usage.input_tokens +
          usage.cache_read_input_tokens +
          usage.cache_write_input_tokens) /
          M) *
          pricing.input_per_million +
        components.outputUsd;
      savings = counterfactual - actual;
      totals.freshInputUsd += components.freshInputUsd;
      totals.cacheWriteUsd += components.cacheWriteUsd;
      totals.cacheReadUsd += components.cacheReadUsd;
      totals.outputUsd += components.outputUsd;
      actualUsd += actual;
      counterfactualUsd += counterfactual;
    } else {
      unpricedTurns += 1;
    }

    entries.push({
      index,
      usage,
      cacheWriteTtl: usage.cache_ttl ?? "five_minutes",
      outcome,
      components,
      actualUsd: actual,
      counterfactualUsd: counterfactual,
      savingsUsd: savings,
    });
    prevMetered = usage;
    prevCompletedAt = completedAt[index] ?? null;
  }

  const savingsUsd = counterfactualUsd - actualUsd;
  return {
    entries,
    unmeteredTurns,
    unpricedTurns,
    totals,
    actualUsd,
    counterfactualUsd,
    savingsUsd,
    savingsPct:
      counterfactualUsd > 0 ? (savingsUsd / counterfactualUsd) * 100 : 0,
    hitCount,
    missCount,
    notReusedCount,
    coldStartCount,
  };
}

function classifyOutcome(
  usage: TurnUsage,
  prevMetered: TurnUsage | null,
  currentCompletedAt: string | null,
  prevCompletedAt: string | null
): CacheOutcome {
  if (usage.cache_read_input_tokens > 0) return { kind: "hit" };
  if (prevMetered === null) return { kind: "cold_start" };
  if (prevMetered.cache_write_input_tokens > 0) {
    const ttl = prevMetered.cache_ttl ?? "five_minutes";
    const previous = prevCompletedAt ? Date.parse(prevCompletedAt) : Number.NaN;
    const current = currentCompletedAt
      ? Date.parse(currentCompletedAt)
      : Number.NaN;
    const ttlMs = ttl === "one_hour" ? 60 * 60_000 : 5 * 60_000;
    if (
      Number.isFinite(previous) &&
      Number.isFinite(current) &&
      current - previous >= ttlMs
    ) {
      return { kind: "miss", expiredTtl: ttl };
    }
    return { kind: "not_reused" };
  }
  return { kind: "no_cache" };
}

/**
 * Presentation-layer savings for banner copy: positive net savings only,
 * with a whole-percent share of the counterfactual. `null` when the
 * session is at break-even or still net-invested in cache writes — the
 * banner then renders exactly as it did before the ledger existed.
 */
export function positiveLedgerSavings(
  ledger: CostLedger
): { usd: number; pct: number } | null {
  if (ledger.savingsUsd <= 0 || ledger.counterfactualUsd <= 0) return null;
  return { usd: ledger.savingsUsd, pct: Math.round(ledger.savingsPct) };
}

/** Short user-facing label for a cache TTL class ("5m" / "1h"). */
export function cacheTtlLabel(ttl: CacheTtlChoice): string {
  return ttl === "one_hour" ? "1h" : "5m";
}
