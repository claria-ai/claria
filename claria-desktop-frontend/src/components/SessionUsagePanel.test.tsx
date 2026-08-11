import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  EMPTY_SESSION_USAGE,
  accumulateUsage,
  type SessionUsage,
} from "../lib/cost";
import { buildCostLedger } from "../lib/costLedger";
import type { ModelPricing, TurnUsage } from "../lib/tauri";
import SessionUsagePanel from "./SessionUsagePanel";

const MODEL = "us.anthropic.claude-sonnet-test";
const pricing = new Map<string, ModelPricing>([
  [
    MODEL,
    {
      input_per_million: 3,
      output_per_million: 15,
      cache_read_per_million: 0.3,
      cache_write_per_million: 3.75,
      cache_write_1h_per_million: 6,
    },
  ],
]);

function usage(over: Partial<TurnUsage>): TurnUsage {
  return {
    model_id: MODEL,
    input_tokens: 100,
    output_tokens: 20,
    cache_read_input_tokens: 0,
    cache_write_input_tokens: 0,
    cache_ttl: "five_minutes",
    cost_usd: 0.01,
    pricing_version: 1,
    ...over,
  };
}

describe("SessionUsagePanel", () => {
  it("shows cache read/write tokens with their associated fees", async () => {
    const turns = [
      usage({ cache_write_input_tokens: 1_000_000, cost_usd: 3.76 }),
      usage({ cache_read_input_tokens: 500_000, cost_usd: 0.16 }),
    ];
    const session = turns.reduce<SessionUsage>(
      (current, turn) => accumulateUsage(current, turn),
      EMPTY_SESSION_USAGE
    );
    const onShowTurnCostsChange = vi.fn();
    render(
      <SessionUsagePanel
        session={session}
        ledger={buildCostLedger(turns, pricing)}
        showTurnCosts={false}
        onShowTurnCostsChange={onShowTurnCostsChange}
      />
    );

    expect(screen.getByText("1000.0k tok")).toBeDefined();
    expect(screen.getByText("$3.75 write fees")).toBeDefined();
    expect(screen.getByText("500.0k tok")).toBeDefined();
    expect(screen.getByText("$0.15 read fees")).toBeDefined();

    await userEvent.click(screen.getByLabelText("Show turn costs"));
    expect(onShowTurnCostsChange).toHaveBeenCalledWith(true);
  });
});
