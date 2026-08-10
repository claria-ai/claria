import { describe, expect, it } from "vitest";
import {
  EMPTY_SESSION_USAGE,
  accumulateUsage,
  cacheHitPct,
  cacheWriteRatePerMillion,
  estimateTurnCost,
  formatCost,
  formatTokens,
  humanRelative,
  prettyModel,
  summarizeHistory,
} from "./cost";
import type { ModelPricing, TurnUsage } from "./tauri";

function usage(over: Partial<TurnUsage> = {}): TurnUsage {
  return {
    model_id: "us.anthropic.claude-sonnet-4-20250514-v1:0",
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

// The rounding rules are spelled out at the top of cost.ts. These are those
// rules, restated as assertions, including both sides of every boundary.
describe("formatCost", () => {
  it("says so when there is no number to show", () => {
    expect(formatCost(null)).toBe("cost unavailable");
    expect(formatCost(undefined)).toBe("cost unavailable");
    expect(formatCost(Number.NaN)).toBe("cost unavailable");
    expect(formatCost(Number.POSITIVE_INFINITY)).toBe("cost unavailable");
  });

  it("treats zero as a real number, not a missing one", () => {
    expect(formatCost(0)).toBe("$0.00");
  });

  it("collapses anything under a cent", () => {
    expect(formatCost(0.000001)).toBe("< $0.01");
    expect(formatCost(0.009999)).toBe("< $0.01");
  });

  it("shows three decimals from a cent up to a dime", () => {
    expect(formatCost(0.01)).toBe("$0.010");
    expect(formatCost(0.0999)).toBe("$0.100");
  });

  it("shows two decimals from a dime up", () => {
    expect(formatCost(0.1)).toBe("$0.10");
    expect(formatCost(12.345)).toBe("$12.35");
  });
});

describe("formatTokens", () => {
  it("groups below ten thousand", () => {
    expect(formatTokens(123)).toBe("123 tok");
    expect(formatTokens(9999)).toBe("9,999 tok");
  });

  it("switches to a k-suffix at ten thousand", () => {
    expect(formatTokens(10_000)).toBe("10.0k tok");
    expect(formatTokens(123_456)).toBe("123.5k tok");
  });

  it("has a dash for nothing", () => {
    expect(formatTokens(null)).toBe("—");
    expect(formatTokens(Number.NaN)).toBe("—");
  });
});

describe("cacheHitPct", () => {
  it("is zero when nothing was billed", () => {
    expect(cacheHitPct(null)).toBe(0);
    expect(cacheHitPct(usage())).toBe(0);
  });

  it("counts cache reads against all input tokens", () => {
    expect(
      cacheHitPct(
        usage({
          input_tokens: 100,
          cache_read_input_tokens: 300,
          cache_write_input_tokens: 100,
        })
      )
    ).toBe(60);
  });

  it("ignores output tokens", () => {
    expect(
      cacheHitPct(
        usage({ input_tokens: 50, cache_read_input_tokens: 50, output_tokens: 9999 })
      )
    ).toBe(50);
  });
});

describe("prettyModel", () => {
  it("strips the scope prefix, the provider and the version stamp", () => {
    expect(prettyModel("us.anthropic.claude-sonnet-4-20250514-v1:0")).toBe(
      "Claude Sonnet 4"
    );
    expect(prettyModel("anthropic.claude-opus-4-20250514-v1:0")).toBe(
      "Claude Opus 4"
    );
    expect(prettyModel("us.anthropic.claude-3-5-haiku-20241022-v1:0")).toBe(
      "Claude 3.5 Haiku"
    );
  });

  it("title-cases anything it does not recognise rather than dropping it", () => {
    expect(prettyModel("meta.llama3-70b-instruct-v1:0")).toBe(
      "Meta.Llama3 70b Instruct V1:0"
    );
  });

  it("has a label for no model at all", () => {
    expect(prettyModel(null)).toBe("Unknown model");
    expect(prettyModel("")).toBe("Unknown model");
  });
});

describe("accumulateUsage", () => {
  it("is a no-op for a turn with no usage recorded", () => {
    expect(accumulateUsage(EMPTY_SESSION_USAGE, null)).toBe(EMPTY_SESSION_USAGE);
    expect(accumulateUsage(EMPTY_SESSION_USAGE, undefined).turnCount).toBe(0);
  });

  it("adds tokens and cost across turns", () => {
    let total = EMPTY_SESSION_USAGE;
    total = accumulateUsage(total, usage({ input_tokens: 10, cost_usd: 0.5 }));
    total = accumulateUsage(total, usage({ input_tokens: 5, cost_usd: 0.25 }));
    expect(total.turnCount).toBe(2);
    expect(total.totalInputTokens).toBe(15);
    expect(total.totalUsd).toBeCloseTo(0.75);
  });

  it("counts an unpriced turn but leaves its cost out of the total", () => {
    const total = accumulateUsage(
      EMPTY_SESSION_USAGE,
      usage({ cost_usd: 99, pricing_version: 0 })
    );
    expect(total.turnCount).toBe(1);
    expect(total.totalUsd).toBe(0);
    expect(total.unknownCostTurns).toBe(1);
  });

  it("values cache reads at the gap between the full and cached input price", () => {
    const pricing = new Map([
      [
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        { input_per_million: 3, cache_read_per_million: 0.3 },
      ],
    ]);
    const total = accumulateUsage(
      EMPTY_SESSION_USAGE,
      usage({ cache_read_input_tokens: 1_000_000 }),
      pricing
    );
    expect(total.cacheSavedUsd).toBeCloseTo(2.7);
  });

  it("claims no savings for a model with no pricing entry", () => {
    const total = accumulateUsage(
      EMPTY_SESSION_USAGE,
      usage({ cache_read_input_tokens: 1_000_000 })
    );
    expect(total.cacheSavedUsd).toBe(0);
  });

  it("never reports a negative saving", () => {
    // Inverted pricing would otherwise subtract from the running total.
    const pricing = new Map([
      [
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        { input_per_million: 0.3, cache_read_per_million: 3 },
      ],
    ]);
    const total = accumulateUsage(
      EMPTY_SESSION_USAGE,
      usage({ cache_read_input_tokens: 1_000_000 }),
      pricing
    );
    expect(total.cacheSavedUsd).toBe(0);
  });

  it("charges cache writes against savings at the turn's TTL rate", () => {
    const pricing = new Map([
      [
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        {
          input_per_million: 3,
          cache_read_per_million: 0.3,
          cache_write_per_million: 3.75,
          cache_write_1h_per_million: 6,
        },
      ],
    ]);
    // 1M reads save (3 − 0.3) = 2.7; 1M five-minute writes cost
    // (3.75 − 3) = 0.75 extra → net 1.95.
    const fiveMinute = accumulateUsage(
      EMPTY_SESSION_USAGE,
      usage({
        cache_read_input_tokens: 1_000_000,
        cache_write_input_tokens: 1_000_000,
        cache_ttl: "five_minutes",
      }),
      pricing
    );
    expect(fiveMinute.cacheSavedUsd).toBeCloseTo(1.95);
    // The same writes at the 1-hour tier cost (6 − 3) = 3 extra → net −0.3,
    // clamped to zero.
    const oneHour = accumulateUsage(
      EMPTY_SESSION_USAGE,
      usage({
        cache_read_input_tokens: 1_000_000,
        cache_write_input_tokens: 1_000_000,
        cache_ttl: "one_hour",
      }),
      pricing
    );
    expect(oneHour.cacheSavedUsd).toBe(0);
  });
});

describe("cacheWriteRatePerMillion", () => {
  const pricing = {
    input_per_million: 3,
    cache_read_per_million: 0.3,
    cache_write_per_million: 3.75,
    cache_write_1h_per_million: 6,
  };

  it("uses the 1-hour rate only for one-hour turns", () => {
    expect(cacheWriteRatePerMillion(pricing, "one_hour")).toBe(6);
    expect(cacheWriteRatePerMillion(pricing, "five_minutes")).toBe(3.75);
  });

  it("defaults to the 5-minute rate when the TTL is absent", () => {
    expect(cacheWriteRatePerMillion(pricing, null)).toBe(3.75);
    expect(cacheWriteRatePerMillion(pricing, undefined)).toBe(3.75);
  });

  it("falls back to the 5-minute rate when the 1-hour rate is missing", () => {
    const legacy = { input_per_million: 3, cache_read_per_million: 0.3, cache_write_per_million: 3.75 };
    expect(cacheWriteRatePerMillion(legacy, "one_hour")).toBe(3.75);
  });
});

describe("estimateTurnCost", () => {
  const pricing: ModelPricing = {
    input_per_million: 3,
    output_per_million: 15,
    cache_read_per_million: 0.3,
    cache_write_per_million: 3.75,
    cache_write_1h_per_million: 6,
  };

  it("is null without pricing, so the caller can hide the estimate", () => {
    expect(estimateTurnCost(null, 1000, 400)).toBeNull();
  });

  it("charges the context plus the pending message at input rates", () => {
    // 1,000,000 context tokens + 4,000,000 chars ≈ 1,000,000 more input
    // tokens, and 200 projected output tokens.
    const estimate = estimateTurnCost(pricing, 1_000_000, 4_000_000);
    expect(estimate).toBeCloseTo(2 * 3 + (200 / 1_000_000) * 15);
  });

  it("takes a caller-supplied projection for the response length", () => {
    const short = estimateTurnCost(pricing, 0, 0, 100);
    const long = estimateTurnCost(pricing, 0, 0, 1000);
    expect(short).toBeCloseTo((100 / 1_000_000) * 15);
    expect(long).toBeCloseTo((1000 / 1_000_000) * 15);
  });

  it("treats a negative context as empty", () => {
    expect(estimateTurnCost(pricing, -50, 0, 0)).toBe(0);
  });
});

describe("humanRelative", () => {
  const now = Date.now();
  const ago = (ms: number) => new Date(now - ms).toISOString();

  it("handles nothing and nonsense", () => {
    expect(humanRelative(null)).toBe("unknown");
    expect(humanRelative("not a date")).toBe("unknown");
  });

  it("counts up through the units", () => {
    expect(humanRelative(ago(5_000))).toBe("just now");
    expect(humanRelative(ago(60_000))).toBe("1 minute ago");
    expect(humanRelative(ago(120_000))).toBe("2 minutes ago");
    expect(humanRelative(ago(3_600_000))).toBe("1 hour ago");
    expect(humanRelative(ago(7_200_000))).toBe("2 hours ago");
    expect(humanRelative(ago(86_400_000))).toBe("yesterday");
    expect(humanRelative(ago(3 * 86_400_000))).toBe("3 days ago");
  });

  it("falls back to a date past a week", () => {
    expect(humanRelative(ago(30 * 86_400_000))).toMatch(/\d{4}/);
  });
});

describe("summarizeHistory", () => {
  it("counts assistant turns only", () => {
    const summary = summarizeHistory([
      { role: "user" },
      { role: "assistant", usage: usage({ cost_usd: 0.5 }) },
      { role: "user" },
      { role: "assistant", usage: usage({ cost_usd: 0.25 }) },
    ]);
    expect(summary.turnCount).toBe(2);
    expect(summary.lifetimeUsd).toBeCloseTo(0.75);
  });

  it("withholds the total rather than understating it", () => {
    // One unpriced turn makes the sum wrong, so there is no figure to show.
    const summary = summarizeHistory([
      { role: "assistant", usage: usage({ cost_usd: 0.5 }) },
      { role: "assistant", usage: usage({ cost_usd: 9, pricing_version: 0 }) },
    ]);
    expect(summary.lifetimeUsd).toBeNull();
    expect(summary.turnCount).toBe(2);
  });

  it("counts turns predating usage tracking separately", () => {
    const summary = summarizeHistory([
      { role: "assistant" },
      { role: "assistant", usage: null },
      { role: "assistant", usage: usage({ cost_usd: 1 }) },
    ]);
    expect(summary.legacyTurnCount).toBe(2);
    expect(summary.turnCount).toBe(3);
    expect(summary.lifetimeUsd).toBeCloseTo(1);
  });

  it("has no total for an empty history", () => {
    const summary = summarizeHistory([]);
    expect(summary.lifetimeUsd).toBeNull();
    expect(summary.turnCount).toBe(0);
  });

  it("passes the last-activity stamp through", () => {
    expect(summarizeHistory([], "2026-01-01T00:00:00Z").lastActivityIso).toBe(
      "2026-01-01T00:00:00Z"
    );
  });
});
