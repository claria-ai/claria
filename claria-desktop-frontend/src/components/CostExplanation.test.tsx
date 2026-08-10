// CostExplanation: renders the ledger rollup and per-turn bars, annotates
// expired cache windows, hides itself without data, and toggles through the
// shared accordion chrome.

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { buildCostLedger } from "../lib/costLedger";
import type { ModelPricing, TurnUsage } from "../lib/tauri";
import CostExplanation from "./CostExplanation";

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

const pricingMap = new Map<string, ModelPricing>([
  [
    SONNET,
    {
      input_per_million: 3,
      output_per_million: 15,
      cache_read_per_million: 0.3,
      cache_write_per_million: 3.75,
      cache_write_1h_per_million: 6,
    },
  ],
]);

describe("CostExplanation", () => {
  it("renders nothing without ledger data", () => {
    const { container } = render(
      <CostExplanation ledger={buildCostLedger([], pricingMap)} />
    );
    expect(container.innerHTML).toBe("");
    const { container: empty } = render(<CostExplanation ledger={null} />);
    expect(empty.innerHTML).toBe("");
  });

  it("renders the session summary and per-turn breakdown bars", () => {
    const ledger = buildCostLedger(
      [
        usage({
          input_tokens: 1_000_000,
          cache_write_input_tokens: 1_000_000,
          cache_ttl: "five_minutes",
        }),
        usage({
          input_tokens: 100_000,
          cache_read_input_tokens: 1_000_000,
          output_tokens: 200_000,
        }),
      ],
      pricingMap
    );
    render(<CostExplanation ledger={ledger} />);

    // Rollup: actual, counterfactual, savings share and hit count. The
    // actual total also appears in the collapsed summary chip.
    expect(screen.getAllByText("$10.35").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("$12.30")).toBeDefined();
    expect(screen.getByText(/Saved \$1\.95 \(16%\)/)).toBeDefined();
    expect(screen.getByText(/Cache hits 1 of 2 turns/)).toBeDefined();

    // Per-turn bars carry exact dollar figures in their labels.
    const rows = within(
      screen.getByRole("list", { name: "Per-turn cost breakdown" })
    ).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(
      screen.getByRole("img", {
        name: "Turn 1 cost $6.75 (Fresh input: $3.00, Cache write (5m): $3.75); without caching $6.00",
      })
    ).toBeDefined();
    expect(within(rows[0]).getByText("cold start")).toBeDefined();
    expect(within(rows[0]).getByText("invested $0.75 in cache")).toBeDefined();
    expect(within(rows[1]).getByText("saved $2.70")).toBeDefined();
  });

  it("annotates a turn that missed an expired cache window", () => {
    const ledger = buildCostLedger(
      [
        usage({ cache_write_input_tokens: 1_000, cache_ttl: "five_minutes" }),
        usage({ input_tokens: 1_000 }),
      ],
      pricingMap
    );
    render(<CostExplanation ledger={ledger} />);
    expect(screen.getByText("5m window expired")).toBeDefined();
  });

  it("labels turns whose model has no pricing entry instead of pricing them at zero", () => {
    const ledger = buildCostLedger(
      [usage({ model_id: "us.anthropic.claude-unknown", input_tokens: 10 })],
      pricingMap
    );
    render(<CostExplanation ledger={ledger} />);
    expect(screen.getByText("no pricing entry for this model")).toBeDefined();
    expect(
      screen.getByText(/1 turn without a pricing entry — totals exclude it/)
    ).toBeDefined();
  });

  it("is collapsed by default and expands from the summary row", async () => {
    const ledger = buildCostLedger([usage({ input_tokens: 10 })], pricingMap);
    render(<CostExplanation ledger={ledger} />);
    const details = screen.getByTestId("cost-explanation") as HTMLDetailsElement;
    expect(details.open).toBe(false);
    await userEvent.click(within(details).getByText("Cost breakdown"));
    expect(details.open).toBe(true);
    await userEvent.click(within(details).getByText("Cost breakdown"));
    expect(details.open).toBe(false);
  });

  it("windows long sessions to the last 50 turns until asked for all", async () => {
    const turns = Array.from({ length: 60 }, () => usage({ input_tokens: 10 }));
    const ledger = buildCostLedger(turns, pricingMap);
    render(<CostExplanation ledger={ledger} />);
    expect(
      within(
        screen.getByRole("list", { name: "Per-turn cost breakdown" })
      ).getAllByRole("listitem")
    ).toHaveLength(50);
    // Numbering keeps global positions: the first rendered row is turn 11.
    expect(screen.getByText("Turn 11")).toBeDefined();
    expect(screen.queryByText("Turn 1")).toBeNull();
    await userEvent.click(
      screen.getByRole("button", { name: "Show all 60 turns (10 hidden)" })
    );
    expect(
      within(
        screen.getByRole("list", { name: "Per-turn cost breakdown" })
      ).getAllByRole("listitem")
    ).toHaveLength(60);
    expect(screen.getByText("Turn 1")).toBeDefined();
  });
});
