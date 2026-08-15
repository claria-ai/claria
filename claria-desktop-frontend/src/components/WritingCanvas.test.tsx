import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ReportDocument } from "./WritingCanvas";
import type { ReportContent } from "../lib/tauri";

const content: ReportContent = {
  title: "Psychoeducational Evaluation",
  sections: [
    {
      id: "11111111-1111-4111-8111-111111111111",
      heading: "Reason for Referral",
      blocks: [{ kind: "paragraph", text: "Referred for attention concerns." }],
      skipped: false,
    },
    {
      id: "22222222-2222-4222-8222-222222222222",
      heading: "Summary and Clinical Interpretation",
      blocks: [],
      skipped: true,
    },
  ],
};

describe("ReportDocument deferred sections", () => {
  it("renders written sections normally and deferred sections as placeholders", () => {
    render(<ReportDocument content={content} />);

    expect(
      screen.getByText("Referred for attention concerns.")
    ).toBeTruthy();

    const deferred = screen.getByTestId("deferred-section");
    expect(deferred.textContent).toContain("Summary and Clinical Interpretation");
    expect(deferred.textContent).toContain("Deferred — not written yet");
    expect(deferred.textContent).toContain("left out of");
  });
});
