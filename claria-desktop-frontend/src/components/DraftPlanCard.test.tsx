import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import DraftPlanCard from "./DraftPlanCard";
import {
  FRESH_INTENTS,
  RESUME_INTENTS,
  type PlanRow,
} from "../lib/draftPlan";

const SECTION_ID = "11111111-1111-4111-8111-111111111111";

function row(overrides: Partial<PlanRow> = {}): PlanRow {
  return {
    sectionId: SECTION_ID,
    heading: "Findings",
    intent: "draft",
    required: true,
    scope: "Summarise the assessment results.",
    evidence: [{ filename: "intake.txt", note: null }],
    instruction: "",
    priorState: null,
    priorError: null,
    ...overrides,
  };
}

function renderCard(overrides: Partial<PlanRow> = {}, props: Partial<{
  intents: readonly PlanRow["intent"][];
  recordFilenames: string[];
}> = {}) {
  const onChange = vi.fn();
  render(
    <DraftPlanCard
      row={row(overrides)}
      intents={props.intents ?? FRESH_INTENTS}
      chip={{ tone: "info", label: "Draft" }}
      recordFilenames={
        props.recordFilenames ?? ["intake.txt", "teacher-observation.txt"]
      }
      onChange={onChange}
    />
  );
  return onChange;
}

describe("DraftPlanCard", () => {
  it("offers a fresh gate two directives and a resume gate four", () => {
    const { unmount } = render(
      <DraftPlanCard
        row={row()}
        intents={FRESH_INTENTS}
        chip={{ tone: "info", label: "Draft" }}
        recordFilenames={[]}
        onChange={vi.fn()}
      />
    );
    expect(
      within(
        screen.getByRole("radiogroup", { name: "Directive for Findings" })
      ).getAllByRole("radio")
    ).toHaveLength(2);
    unmount();

    render(
      <DraftPlanCard
        row={row({ priorState: "drafted", intent: "keep" })}
        intents={RESUME_INTENTS}
        chip={{ tone: "neutral", label: "Keep" }}
        recordFilenames={[]}
        onChange={vi.fn()}
      />
    );
    const group = screen.getByRole("radiogroup", {
      name: "Directive for Findings",
    });
    expect(within(group).getAllByRole("radio")).toHaveLength(4);
    expect(
      within(group).getByRole("radio", { name: "Keep" }).getAttribute(
        "aria-checked"
      )
    ).toBe("true");
  });

  it("reports the directive the reader picked", async () => {
    const onChange = renderCard();

    await userEvent.click(screen.getByRole("radio", { name: "Skip" }));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ sectionId: SECTION_ID, intent: "skip" })
    );
  });

  it("adds and removes evidence by filename", async () => {
    const onChange = renderCard();

    await userEvent.click(
      screen.getByRole("button", { name: "Add evidence to Findings" })
    );
    // A record already named is not offered a second time.
    const picker = screen.getByLabelText("Records available to Findings");
    expect(within(picker).queryByRole("button", { name: "intake.txt" })).toBeNull();
    await userEvent.click(
      within(picker).getByRole("button", { name: "teacher-observation.txt" })
    );
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({
        evidence: [
          { filename: "intake.txt", note: null },
          { filename: "teacher-observation.txt", note: null },
        ],
      })
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Remove intake.txt from Findings" })
    );
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ evidence: [] })
    );
  });

  it("says so when every record is already named", async () => {
    renderCard({}, { recordFilenames: ["intake.txt"] });

    await userEvent.click(
      screen.getByRole("button", { name: "Add evidence to Findings" })
    );

    expect(
      screen.getByText("Every record this client has is already named here.")
    ).toBeDefined();
  });

  it("carries the scope and the per-section instruction back out", async () => {
    const onChange = renderCard({ scope: "", instruction: "" });

    await userEvent.type(screen.getByLabelText("Scope for Findings"), "A");
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ scope: "A" })
    );

    await userEvent.type(
      screen.getByLabelText("Instructions for Findings"),
      "B"
    );
    expect(onChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ instruction: "B" })
    );
  });

  it("shows a failed section's reason and offers no edits while a run owns it", () => {
    render(
      <DraftPlanCard
        row={row({ priorState: "failed", priorError: "section_attempts_exhausted" })}
        intents={[]}
        chip={{ tone: "progress", label: "Writing", animated: true }}
        recordFilenames={[]}
        onChange={vi.fn()}
      />
    );

    expect(screen.getByText("Failed: section_attempts_exhausted")).toBeDefined();
    expect(screen.queryByRole("radiogroup")).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Add evidence to Findings" })
    ).toBeNull();
    expect(
      (screen.getByLabelText("Scope for Findings") as HTMLTextAreaElement)
        .readOnly
    ).toBe(true);
  });
});
