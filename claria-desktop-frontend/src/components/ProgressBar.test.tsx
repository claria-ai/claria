import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import ProgressBar from "./ProgressBar";

function fillWidth(): number {
  const track = screen.getByRole("progressbar");
  const fill = track.firstElementChild as HTMLElement;
  return Number.parseFloat(fill.style.width);
}

describe("ProgressBar", () => {
  it("exposes the value through progressbar ARIA attributes", () => {
    render(
      <ProgressBar value={4} max={9} label="Downloading model" valueText="44%" />
    );

    const track = screen.getByRole("progressbar");
    expect(track.getAttribute("aria-label")).toBe("Downloading model");
    expect(track.getAttribute("aria-valuemin")).toBe("0");
    expect(track.getAttribute("aria-valuemax")).toBe("9");
    expect(track.getAttribute("aria-valuenow")).toBe("4");
    expect(track.getAttribute("aria-valuetext")).toBe("44%");
  });

  it("sizes the fill as a percentage of max", () => {
    render(<ProgressBar value={4} max={9} label="Progress" valueText="44%" />);

    expect(fillWidth()).toBeCloseTo(44.44, 2);
  });

  it("clamps a value above max to a full bar", () => {
    render(<ProgressBar value={15} max={10} label="Progress" valueText="100%" />);

    expect(fillWidth()).toBe(100);
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe(
      "10"
    );
  });

  it("clamps a negative value to an empty bar", () => {
    render(<ProgressBar value={-5} max={10} label="Progress" valueText="0%" />);

    expect(fillWidth()).toBe(0);
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe(
      "0"
    );
  });

  it("renders an empty bar rather than NaN when max is zero", () => {
    render(<ProgressBar value={3} max={0} label="Progress" valueText="0%" />);

    const track = screen.getByRole("progressbar");
    expect(track.firstElementChild?.getAttribute("style")).toContain("width: 0%");
    expect(fillWidth()).toBe(0);
    expect(track.getAttribute("aria-valuenow")).toBe("0");
    expect(track.getAttribute("aria-valuemax")).toBe("0");
  });

  it("renders the value text under the bar by default", () => {
    render(
      <ProgressBar value={2} max={4} label="Progress" valueText="512 MB of 1 GB" />
    );

    expect(screen.getByText("512 MB of 1 GB")).toBeTruthy();
  });

  it("keeps the value text for assistive tech when it is hidden", () => {
    render(
      <ProgressBar
        value={2}
        max={4}
        label="Progress"
        valueText="50%"
        showValueText={false}
      />
    );

    expect(screen.queryByText("50%")).toBeNull();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuetext")).toBe(
      "50%"
    );
  });
});
