import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import FindingCard from "./FindingCard";
import type { Finding } from "../lib/tauri";
import type { FindingState } from "../lib/findings";

const SECTION_ID = "11111111-1111-4111-8111-111111111111";
const OTHER_SECTION_ID = "22222222-2222-4222-8222-222222222222";

function styleFinding(overrides: Partial<Finding> = {}): Finding {
  return {
    id: "f-style",
    pass: "style",
    property: "tense_drift",
    model_id: "model-1",
    anchor: { section_id: SECTION_ID, revision: 3 },
    description: "The paragraph slips from past to present tense.",
    span: { block_index: 0, start_char: 0, end_char: 12 },
    conflicting: null,
    record_citation: null,
    proposal: {
      block_index: 0,
      original_text: "Jane presents as attentive.",
      replacement_text: "Jane presented as attentive.",
    },
    status: "open",
    applied_revision: null,
    resolved_at: null,
    created_at: "2026-08-01T00:00:00Z",
    ...overrides,
  };
}

function consistencyFinding(overrides: Partial<Finding> = {}): Finding {
  return {
    id: "f-consistency",
    pass: "consistency",
    property: "cross_section_conflict",
    model_id: "model-1",
    anchor: { section_id: SECTION_ID, revision: 3 },
    description: "The reported age does not match the Background section.",
    span: { block_index: 1, start_char: 0, end_char: 20 },
    conflicting: {
      section_id: OTHER_SECTION_ID,
      quote: "Jane is nine years old.",
      span: null,
    },
    record_citation: { filename: "intake.txt", quote: "age 11" },
    proposal: null,
    status: "open",
    applied_revision: null,
    resolved_at: null,
    created_at: "2026-08-01T00:00:00Z",
    ...overrides,
  };
}

function renderCard(
  finding: Finding,
  state: FindingState,
  overrides: Partial<{
    quote: string | null;
    conflictingHeading: string | null;
    currentRevision: number;
    busy: boolean;
    resolving: boolean;
  }> = {}
) {
  const handlers = {
    onApply: vi.fn(),
    onUndo: vi.fn(),
    onDismiss: vi.fn(),
    onReference: vi.fn(),
    onPreviewRecord: vi.fn(),
  };
  render(
    <FindingCard
      finding={finding}
      state={state}
      quote={overrides.quote ?? null}
      conflictingHeading={overrides.conflictingHeading ?? null}
      currentRevision={overrides.currentRevision ?? 5}
      busy={overrides.busy ?? false}
      resolving={overrides.resolving ?? false}
      {...handlers}
    />
  );
  return handlers;
}

describe("FindingCard", () => {
  it("offers a style finding its replacement and applies it", async () => {
    const handlers = renderCard(styleFinding(), "open");

    expect(screen.getByText("Current")).toBeDefined();
    expect(screen.getByText("Proposed")).toBeDefined();
    expect(screen.getByText("Jane presents as attentive.")).toBeDefined();
    expect(screen.getByText("Jane presented as attentive.")).toBeDefined();
    expect(screen.getByText("tense drift")).toBeDefined();

    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(handlers.onApply).toHaveBeenCalledWith("f-style");
  });

  it("collapses an applied finding to a receipt that undoes it", async () => {
    const handlers = renderCard(
      styleFinding({ status: "applied", applied_revision: 7 }),
      "applied"
    );

    expect(screen.getByTestId("finding-receipt").textContent).toContain(
      "Applied in r7"
    );
    expect(screen.queryByRole("button", { name: "Apply" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull();
    // The replacement is spent, so the comparison goes with it.
    expect(screen.queryByText("Current")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(handlers.onUndo).toHaveBeenCalledWith("f-style");
  });

  it("never offers a consistency finding a fix", () => {
    renderCard(consistencyFinding(), "open", {
      quote: "Jane is eleven years old.",
      conflictingHeading: "Background",
    });

    expect(screen.queryByRole("button", { name: "Apply" })).toBeNull();
    expect(screen.queryByText("Proposed")).toBeNull();
    expect(screen.getByTestId("finding-quote").textContent).toBe(
      "Jane is eleven years old."
    );
    expect(screen.getByTestId("finding-conflict").textContent).toContain(
      "conflicts with Background"
    );
    expect(screen.getByTestId("finding-conflict").textContent).toContain(
      "Jane is nine years old."
    );
    expect(screen.getByRole("button", { name: "Dismiss" })).toBeDefined();
    expect(
      screen.getByRole("button", { name: "Reference in chat" })
    ).toBeDefined();
  });

  it("hands the anchored block to the composer", async () => {
    const handlers = renderCard(consistencyFinding(), "open");

    await userEvent.click(
      screen.getByRole("button", { name: "Reference in chat" })
    );

    expect(handlers.onReference).toHaveBeenCalledWith({
      sectionId: SECTION_ID,
      blockIndex: 1,
    });
  });

  it("falls back to the section alone when the finding resolved to no block", async () => {
    const handlers = renderCard(
      consistencyFinding({ id: "f-nospan", span: null }),
      "open"
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Reference in chat" })
    );

    expect(handlers.onReference).toHaveBeenCalledWith({
      sectionId: SECTION_ID,
      blockIndex: null,
    });
  });

  it("opens the cited record from its filename pill", async () => {
    const handlers = renderCard(consistencyFinding(), "open");

    await userEvent.click(screen.getByRole("button", { name: "intake.txt" }));
    expect(handlers.onPreviewRecord).toHaveBeenCalledWith("intake.txt");
  });

  it("makes an invalidated finding inert apart from dismissing it", async () => {
    const handlers = renderCard(styleFinding(), "invalidated", {
      currentRevision: 9,
    });

    const card = screen.getByTestId("finding-card");
    expect(card.getAttribute("data-finding-state")).toBe("invalidated");
    expect(card.textContent).toContain(
      "This section changed after the review (r3 → r9)."
    );
    expect(card.textContent).toContain("Re-run the review to refresh.");
    expect(
      screen.getByText("The paragraph slips from past to present tense.")
        .className
    ).toContain("line-through");
    expect(screen.queryByRole("button", { name: "Apply" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Reference in chat" })
    ).toBeNull();
    expect(screen.queryByText("Current")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(handlers.onDismiss).toHaveBeenCalledWith("f-style");
  });

  it("goes quiet while another writer action is in flight", () => {
    renderCard(styleFinding(), "open", { busy: true, resolving: true });

    expect(
      (screen.getByRole("button", { name: "Applying…" }) as HTMLButtonElement)
        .disabled
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: "Dismiss" }) as HTMLButtonElement)
        .disabled
    ).toBe(true);
  });
});
