// SessionTotalBanner: the optional ledger-fed savings line appears only
// when positive savings are passed in; without it the banner is unchanged.

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { EMPTY_SESSION_USAGE, type SessionUsage } from "../lib/cost";
import SessionTotalBanner from "./SessionTotalBanner";

const SESSION: SessionUsage = {
  ...EMPTY_SESSION_USAGE,
  totalUsd: 1.25,
  totalInputTokens: 1000,
  totalOutputTokens: 500,
  turnCount: 3,
};

describe("SessionTotalBanner", () => {
  it("shows the caching-savings line when the ledger reports savings", () => {
    render(
      <SessionTotalBanner
        session={SESSION}
        cacheSavings={{ usd: 0.42, pct: 34 }}
      />
    );
    expect(screen.getByText("Caching saved $0.42 (34%)")).toBeDefined();
  });

  it("renders exactly as before when no savings data exists", () => {
    render(<SessionTotalBanner session={SESSION} cacheSavings={null} />);
    expect(screen.queryByText(/Caching saved/)).toBeNull();
    expect(screen.getByText("$1.25")).toBeDefined();
  });
});
