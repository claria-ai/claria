import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import FindingsList from "./FindingsList";
import type { Finding, ReportContent } from "../lib/tauri";

const FINDINGS_ID = "11111111-1111-4111-8111-111111111111";
const BACKGROUND_ID = "22222222-2222-4222-8222-222222222222";
const GONE_ID = "33333333-3333-4333-8333-333333333333";

/** Background comes first in the document, Findings second. */
function content(): ReportContent {
  return {
    title: "Evaluation",
    sections: [
      {
        id: BACKGROUND_ID,
        heading: "Background",
        blocks: [{ kind: "paragraph", text: "Jane is eleven years old." }],
      },
      {
        id: FINDINGS_ID,
        heading: "Findings",
        blocks: [{ kind: "paragraph", text: "Jane presents as attentive." }],
      },
    ],
  };
}

function finding(overrides: Partial<Finding> & { id: string }): Finding {
  return {
    pass: "style",
    property: "tense_drift",
    model_id: "model-1",
    anchor: { section_id: FINDINGS_ID, revision: 1 },
    description: `Description for ${overrides.id}`,
    span: null,
    conflicting: null,
    record_citation: null,
    proposal: {
      block_index: 0,
      original_text: "presents",
      replacement_text: "presented",
    },
    status: "open",
    applied_revision: null,
    resolved_at: null,
    created_at: "2026-08-01T00:00:00Z",
    ...overrides,
  };
}

const ROWS: Finding[] = [
  finding({ id: "style-findings" }),
  finding({
    id: "consistency-background",
    pass: "consistency",
    property: "internal_contradiction",
    anchor: { section_id: BACKGROUND_ID, revision: 1 },
    proposal: null,
  }),
  finding({
    id: "applied-findings",
    status: "applied",
    applied_revision: 4,
  }),
];

function renderList(rows: Finding[] = ROWS, sections = content()) {
  const registerSection = vi.fn();
  render(
    <FindingsList
      findings={rows}
      content={sections}
      draftRevision={4}
      busy={false}
      resolvingId={null}
      onApply={vi.fn()}
      onUndo={vi.fn()}
      onDismiss={vi.fn()}
      onReference={vi.fn()}
      onPreviewRecord={vi.fn()}
      registerSection={registerSection}
    />
  );
  return registerSection;
}

function groupHeadings(): string[] {
  return Array.from(
    document.querySelectorAll("[data-finding-section] h4")
  ).map((heading) => heading.textContent ?? "");
}

describe("FindingsList", () => {
  it("groups by section in document order and counts what is still open", () => {
    renderList();

    expect(groupHeadings()).toEqual(["Background", "Findings"]);
    const findingsGroup = document.querySelector(
      `[data-finding-section="${FINDINGS_ID}"]`
    ) as HTMLElement;
    // Two findings under Findings, but only one of them still open.
    expect(within(findingsGroup).getAllByTestId("finding-card")).toHaveLength(2);
    expect(findingsGroup.textContent).toContain("1 open");
  });

  it("filters by pass and by resolution without moving the open counts", async () => {
    renderList();

    await userEvent.click(screen.getByRole("button", { name: "Consistency" }));
    expect(groupHeadings()).toEqual(["Background"]);

    await userEvent.click(screen.getByRole("button", { name: "Style" }));
    expect(groupHeadings()).toEqual(["Findings"]);
    expect(screen.getAllByTestId("finding-card")).toHaveLength(2);

    await userEvent.click(screen.getByRole("button", { name: "Resolved" }));
    const resolved = screen.getAllByTestId("finding-card");
    expect(resolved).toHaveLength(1);
    expect(resolved[0].getAttribute("data-finding-id")).toBe("applied-findings");
    // The section's open count is counted before the filter runs.
    expect(
      (
        document.querySelector(
          `[data-finding-section="${FINDINGS_ID}"]`
        ) as HTMLElement
      ).textContent
    ).toContain("1 open");

    await userEvent.click(screen.getByRole("button", { name: "All" }));
    expect(groupHeadings()).toEqual(["Background", "Findings"]);
  });

  it("draws a finding whose section moved on as invalidated", () => {
    const sections = content();
    sections.sections[1].authorship = {
      kind: "human_edited",
      revision: 3,
      model_id: null,
      run_id: null,
      updated_at: "2026-08-02T00:00:00Z",
    };
    renderList([finding({ id: "stale" })], sections);

    const card = screen.getByTestId("finding-card");
    expect(card.getAttribute("data-finding-state")).toBe("invalidated");
    expect(card.textContent).toContain("(r1 → r3)");
    expect(
      (
        document.querySelector(
          `[data-finding-section="${FINDINGS_ID}"]`
        ) as HTMLElement
      ).textContent
    ).toContain("0 open");
  });

  it("keeps a finding whose section is gone reachable rather than dropping it", () => {
    renderList([
      finding({ id: "orphan", anchor: { section_id: GONE_ID, revision: 1 } }),
    ]);

    expect(groupHeadings()).toEqual(["Section no longer in the report"]);
    expect(screen.getByTestId("finding-card").getAttribute("data-finding-state")).toBe(
      "invalidated"
    );
  });

  it("registers each group so the canvas flag chips can scroll to it", () => {
    const registerSection = renderList();

    expect(registerSection).toHaveBeenCalledWith(
      FINDINGS_ID,
      expect.any(Object)
    );
    expect(registerSection).toHaveBeenCalledWith(
      BACKGROUND_ID,
      expect.any(Object)
    );
  });

  it("says so when a filter matches nothing", async () => {
    renderList([finding({ id: "only-style" })]);

    await userEvent.click(screen.getByRole("button", { name: "Consistency" }));

    expect(screen.getByText("Nothing to show under this filter.")).toBeDefined();
    expect(screen.queryByTestId("finding-card")).toBeNull();
  });
});
