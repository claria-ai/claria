import { describe, expect, it } from "vitest";
import {
  buildCostLedger,
  cacheTtlLabel,
  positiveLedgerSavings,
} from "./costLedger";
import type { ModelPricing, TurnUsage } from "./tauri";

const SONNET = "us.anthropic.claude-sonnet-4-20250514-v1:0";

function usage(over: Partial<TurnUsage> = {}): TurnUsage {
  return {
    model_id: SONNET,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_input_tokens: 0,
    cache_write_input_tokens: 0,
    cache_ttl: null,
    cost_usd: 0,
    pricing_version: 1,
    ...over,
  };
}

const PRICING: ModelPricing = {
  input_per_million: 3,
  output_per_million: 15,
  cache_read_per_million: 0.3,
  cache_write_per_million: 3.75,
  cache_write_1h_per_million: 6,
};

const pricingMap = new Map([[SONNET, PRICING]]);

describe("buildCostLedger", () => {
  it("is empty for an empty session", () => {
    const ledger = buildCostLedger([], pricingMap);
    expect(ledger.entries).toEqual([]);
    expect(ledger.actualUsd).toBe(0);
    expect(ledger.counterfactualUsd).toBe(0);
    expect(ledger.savingsUsd).toBe(0);
    expect(ledger.savingsPct).toBe(0);
    expect(ledger.unmeteredTurns).toBe(0);
  });

  it("prices each component at its real rate", () => {
    const ledger = buildCostLedger(
      [
        usage({
          input_tokens: 100_000,
          cache_write_input_tokens: 1_000_000,
          cache_read_input_tokens: 2_000_000,
          output_tokens: 200_000,
          cache_ttl: "five_minutes",
        }),
      ],
      pricingMap
    );
    const [entry] = ledger.entries;
    expect(entry.components).not.toBeNull();
    expect(entry.components?.freshInputUsd).toBeCloseTo(0.3);
    expect(entry.components?.cacheWriteUsd).toBeCloseTo(3.75);
    expect(entry.components?.cacheReadUsd).toBeCloseTo(0.6);
    expect(entry.components?.outputUsd).toBeCloseTo(3);
    // Actual is the sum of the components; the counterfactual bills every
    // input token at the full rate.
    expect(entry.actualUsd).toBeCloseTo(7.65);
    expect(entry.counterfactualUsd).toBeCloseTo(3.1 * 3 + 3);
    expect(entry.savingsUsd).toBeCloseTo(12.3 - 7.65);
  });

  it("keeps negative savings signed for cache-investment turns", () => {
    // A cold write-heavy first turn costs more than the uncached
    // counterfactual — the ledger reports that honestly, no clamping.
    const ledger = buildCostLedger(
      [
        usage({
          input_tokens: 1_000_000,
          cache_write_input_tokens: 1_000_000,
          cache_ttl: "five_minutes",
        }),
      ],
      pricingMap
    );
    expect(ledger.entries[0].savingsUsd).toBeCloseTo(-0.75);
    expect(ledger.savingsUsd).toBeCloseTo(-0.75);
    expect(ledger.entries[0].outcome).toEqual({ kind: "cold_start" });
  });

  it("prices mixed TTLs turn by turn", () => {
    const ledger = buildCostLedger(
      [
        usage({ cache_write_input_tokens: 1_000_000, cache_ttl: "five_minutes" }),
        usage({
          cache_read_input_tokens: 1_000_000,
          cache_write_input_tokens: 1_000_000,
          cache_ttl: "one_hour",
        }),
      ],
      pricingMap
    );
    expect(ledger.entries[0].components?.cacheWriteUsd).toBeCloseTo(3.75);
    expect(ledger.entries[0].cacheWriteTtl).toBe("five_minutes");
    expect(ledger.entries[1].components?.cacheWriteUsd).toBeCloseTo(6);
    expect(ledger.entries[1].cacheWriteTtl).toBe("one_hour");
  });

  it("bills legacy turns without a TTL at the 5-minute rate", () => {
    const legacy = usage({ cache_write_input_tokens: 1_000_000 });
    delete (legacy as Partial<TurnUsage>).cache_ttl;
    const ledger = buildCostLedger([legacy], pricingMap);
    expect(ledger.entries[0].components?.cacheWriteUsd).toBeCloseTo(3.75);
    expect(ledger.entries[0].cacheWriteTtl).toBe("five_minutes");
  });

  it("falls back to the 5-minute write rate when the 1-hour price is missing", () => {
    const legacyPricing = new Map([
      [
        SONNET,
        {
          input_per_million: 3,
          output_per_million: 15,
          cache_read_per_million: 0.3,
          cache_write_per_million: 3.75,
        } as ModelPricing,
      ],
    ]);
    const ledger = buildCostLedger(
      [usage({ cache_write_input_tokens: 1_000_000, cache_ttl: "one_hour" })],
      legacyPricing
    );
    expect(ledger.entries[0].components?.cacheWriteUsd).toBeCloseTo(3.75);
    // The window class is still the one the turn ran with.
    expect(ledger.entries[0].cacheWriteTtl).toBe("one_hour");
  });

  it("marks the first metered turn as a cold start, not a miss", () => {
    const ledger = buildCostLedger(
      [null, usage({ input_tokens: 10 })],
      pricingMap
    );
    expect(ledger.entries[0].outcome).toEqual({ kind: "cold_start" });
    expect(ledger.coldStartCount).toBe(1);
    expect(ledger.missCount).toBe(0);
    // The unmetered leading turn kept its slot in the index.
    expect(ledger.entries[0].index).toBe(1);
  });

  it("attributes a miss to the window the predecessor wrote", () => {
    const ledger = buildCostLedger(
      [
        usage({ cache_write_input_tokens: 100, cache_ttl: "one_hour" }),
        usage({ input_tokens: 10 }),
        usage({ cache_write_input_tokens: 100, cache_ttl: null }),
        usage({ input_tokens: 10 }),
      ],
      pricingMap
    );
    expect(ledger.entries[1].outcome).toEqual({
      kind: "miss",
      expiredTtl: "one_hour",
    });
    // Absent TTL on the predecessor means the 5-minute default expired.
    expect(ledger.entries[3].outcome).toEqual({
      kind: "miss",
      expiredTtl: "five_minutes",
    });
    expect(ledger.missCount).toBe(2);
  });

  it("does not call it a miss when the predecessor wrote nothing", () => {
    const ledger = buildCostLedger(
      [usage({ input_tokens: 10 }), usage({ input_tokens: 10 })],
      pricingMap
    );
    expect(ledger.entries[1].outcome).toEqual({ kind: "no_cache" });
    expect(ledger.missCount).toBe(0);
  });

  it("counts cache reads as hits", () => {
    const ledger = buildCostLedger(
      [
        usage({ cache_write_input_tokens: 100 }),
        usage({ cache_read_input_tokens: 100 }),
      ],
      pricingMap
    );
    expect(ledger.entries[1].outcome).toEqual({ kind: "hit" });
    expect(ledger.hitCount).toBe(1);
  });

  it("counts unmetered turns separately, never as zeros", () => {
    const ledger = buildCostLedger(
      [usage({ input_tokens: 1_000_000 }), null, undefined],
      pricingMap
    );
    expect(ledger.unmeteredTurns).toBe(2);
    expect(ledger.entries).toHaveLength(1);
    expect(ledger.actualUsd).toBeCloseTo(3);
  });

  it("excludes unpriced models from the totals and counts them", () => {
    const ledger = buildCostLedger(
      [
        usage({ input_tokens: 1_000_000 }),
        usage({ model_id: "us.anthropic.claude-unknown", input_tokens: 5_000_000 }),
      ],
      pricingMap
    );
    expect(ledger.unpricedTurns).toBe(1);
    expect(ledger.entries[1].components).toBeNull();
    expect(ledger.entries[1].actualUsd).toBeNull();
    expect(ledger.entries[1].savingsUsd).toBeNull();
    expect(ledger.actualUsd).toBeCloseTo(3);
    expect(ledger.counterfactualUsd).toBeCloseTo(3);
  });

  it("rolls up totals, cumulative savings and the savings share", () => {
    const ledger = buildCostLedger(
      [
        // Cold start: 1M fresh + 1M written → actual 6.75, uncached 6.
        usage({
          input_tokens: 1_000_000,
          cache_write_input_tokens: 1_000_000,
          cache_ttl: "five_minutes",
        }),
        // Hit: 0.1M fresh + 1M read + 0.2M out → actual 3.6, uncached 6.3.
        usage({
          input_tokens: 100_000,
          cache_read_input_tokens: 1_000_000,
          output_tokens: 200_000,
          cache_ttl: "five_minutes",
        }),
      ],
      pricingMap
    );
    expect(ledger.totals.freshInputUsd).toBeCloseTo(3.3);
    expect(ledger.totals.cacheWriteUsd).toBeCloseTo(3.75);
    expect(ledger.totals.cacheReadUsd).toBeCloseTo(0.3);
    expect(ledger.totals.outputUsd).toBeCloseTo(3);
    expect(ledger.actualUsd).toBeCloseTo(10.35);
    expect(ledger.counterfactualUsd).toBeCloseTo(12.3);
    expect(ledger.savingsUsd).toBeCloseTo(1.95);
    expect(ledger.savingsPct).toBeCloseTo((1.95 / 12.3) * 100);
    expect(ledger.coldStartCount).toBe(1);
    expect(ledger.hitCount).toBe(1);
  });
});

describe("positiveLedgerSavings", () => {
  it("passes positive savings through with a whole-percent share", () => {
    const ledger = buildCostLedger(
      [
        usage({ cache_write_input_tokens: 100_000, cache_ttl: "five_minutes" }),
        usage({ cache_read_input_tokens: 1_000_000 }),
      ],
      pricingMap
    );
    const savings = positiveLedgerSavings(ledger);
    expect(savings).not.toBeNull();
    expect(savings?.usd).toBeCloseTo(ledger.savingsUsd);
    expect(savings?.pct).toBe(Math.round(ledger.savingsPct));
  });

  it("is null while the session is net-invested in cache writes", () => {
    const ledger = buildCostLedger(
      [usage({ cache_write_input_tokens: 1_000_000, cache_ttl: "one_hour" })],
      pricingMap
    );
    expect(ledger.savingsUsd).toBeLessThan(0);
    expect(positiveLedgerSavings(ledger)).toBeNull();
  });

  it("is null for an empty ledger", () => {
    expect(positiveLedgerSavings(buildCostLedger([], pricingMap))).toBeNull();
  });
});

describe("cacheTtlLabel", () => {
  it("labels both window classes", () => {
    expect(cacheTtlLabel("five_minutes")).toBe("5m");
    expect(cacheTtlLabel("one_hour")).toBe("1h");
  });
});
