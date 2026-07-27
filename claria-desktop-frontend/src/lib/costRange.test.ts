import { describe, expect, it } from "vitest";
import { daysBetween, defaultGranularity } from "./costRange";

describe("daysBetween", () => {
  it("counts whole days", () => {
    expect(daysBetween("2026-03-01", "2026-03-08")).toBe(7);
  });

  it("is zero for the same day", () => {
    expect(daysBetween("2026-03-01", "2026-03-01")).toBe(0);
  });

  it("does not care which date comes first", () => {
    expect(daysBetween("2026-03-08", "2026-03-01")).toBe(7);
  });

  it("crosses month and year boundaries", () => {
    expect(daysBetween("2026-01-31", "2026-02-01")).toBe(1);
    expect(daysBetween("2025-12-31", "2026-01-01")).toBe(1);
  });

  it("counts the leap day", () => {
    expect(daysBetween("2028-02-28", "2028-03-01")).toBe(2);
    expect(daysBetween("2027-02-28", "2027-03-01")).toBe(1);
  });

  it("rounds across a daylight-saving shift rather than reporting a fraction", () => {
    // US DST begins 2026-03-08. A 23-hour day would otherwise give 6.96.
    expect(daysBetween("2026-03-01", "2026-03-08")).toBe(7);
    expect(Number.isInteger(daysBetween("2026-03-01", "2026-03-08"))).toBe(true);
  });
});

describe("defaultGranularity", () => {
  it("is hourly up to a fortnight", () => {
    expect(defaultGranularity("2026-03-01", "2026-03-01")).toBe("hourly");
    expect(defaultGranularity("2026-03-01", "2026-03-15")).toBe("hourly");
  });

  it("steps down to daily past a fortnight", () => {
    expect(defaultGranularity("2026-03-01", "2026-03-16")).toBe("daily");
  });

  it("stays daily to the ninety-day mark", () => {
    expect(defaultGranularity("2026-01-01", "2026-04-01")).toBe("daily");
  });

  it("steps down to monthly past ninety days", () => {
    expect(defaultGranularity("2026-01-01", "2026-04-02")).toBe("monthly");
    expect(defaultGranularity("2025-01-01", "2026-01-01")).toBe("monthly");
  });

  it("falls back to daily on a short range when the account has no hourly data", () => {
    expect(defaultGranularity("2026-03-01", "2026-03-08", false)).toBe("daily");
  });

  it("is unaffected by the hourly flag past a fortnight", () => {
    expect(defaultGranularity("2026-01-01", "2026-04-02", false)).toBe("monthly");
  });
});
