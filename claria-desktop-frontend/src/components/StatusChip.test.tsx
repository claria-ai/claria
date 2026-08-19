import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import StatusChip, { type StatusChipTone } from "./StatusChip";

const TONES: StatusChipTone[] = [
  "neutral",
  "info",
  "progress",
  "success",
  "danger",
  "warning",
  "muted",
];

describe("StatusChip", () => {
  it("gives every tone its own complete class literals", () => {
    const seen = new Set<string>();
    for (const tone of TONES) {
      const { unmount } = render(<StatusChip tone={tone} label={tone} />);
      const chip = screen.getByTestId("status-chip");
      expect(chip.getAttribute("data-tone")).toBe(tone);
      // Assembled class names compile to nothing, so each tone must carry
      // whole border, background, and text literals.
      expect(chip.className).toMatch(/\bborder-[a-z]+-\d{2,3}\b|\bborder-gray-200\b/);
      expect(chip.className).toMatch(/\bbg-[a-z]+(-\d{2,3})?\b/);
      expect(chip.className).toMatch(/\btext-[a-z]+-\d{2,3}\b/);
      seen.add(chip.className);
      unmount();
    }
    expect(seen.size).toBe(TONES.length);
  });

  it("carries no margins of its own, so any surface can place it", () => {
    render(<StatusChip tone="info" label="Writing" />);
    expect(screen.getByTestId("status-chip").className).not.toMatch(/\bm[trblxy]?-/);
  });

  it("pulses only while asked to", () => {
    const { unmount } = render(<StatusChip tone="progress" label="Writing" />);
    expect(
      screen.getByTestId("status-chip").querySelector(".animate-pulse")
    ).toBeNull();
    unmount();

    render(<StatusChip tone="progress" label="Writing" animated />);
    expect(
      screen.getByTestId("status-chip").querySelector(".animate-pulse")
    ).toBeTruthy();
  });
});
