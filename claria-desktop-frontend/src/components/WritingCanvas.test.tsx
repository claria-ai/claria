import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import WritingCanvas, { ReportDocument } from "./WritingCanvas";
import type {
  ReportBlock,
  ReportContent,
  ReportDraftEdit,
  ReportSection,
  ReportWorkspaceView,
} from "../lib/tauri";
import {
  emptyDraftRun,
  type DraftRunUiState,
  type SectionUiState,
} from "../lib/draftRun";
import { draftToEdit } from "../lib/writingWorkspace";

const WRITTEN_SECTION_ID = "11111111-1111-4111-8111-111111111111";
const SKIPPED_SECTION_ID = "22222222-2222-4222-8222-222222222222";

const templateBlocks: ReportBlock[] = [
  { kind: "paragraph", text: "Template summary paragraph." },
  { kind: "bullet_list", items: ["Template bullet"] },
  {
    kind: "table",
    rows: [
      ["Measure", "Score"],
      ["Attention", "87"],
    ],
    has_header: true,
    column_widths: null,
  },
];

function skippedSection(overrides: Partial<ReportSection> = {}): ReportSection {
  return {
    id: SKIPPED_SECTION_ID,
    heading: "Summary and Clinical Interpretation",
    blocks: [],
    skipped: true,
    template_blocks: templateBlocks,
    ...overrides,
  };
}

function reportContent(sections: ReportSection[]): ReportContent {
  return { title: "Psychoeducational Evaluation", sections };
}

const writtenSection: ReportSection = {
  id: WRITTEN_SECTION_ID,
  heading: "Reason for Referral",
  blocks: [{ kind: "paragraph", text: "Referred for attention concerns." }],
  skipped: false,
};

function workspace(content: ReportContent): ReportWorkspaceView {
  return {
    schema_version: 7,
    report_id: "report-1",
    client_id: "client-1",
    session_name: "Evaluation",
    draft: {
      revision: 4,
      content,
      created_at: "2026-08-01T00:00:00Z",
      updated_at: "2026-08-01T00:00:00Z",
      last_applied_proposal_id: null,
    },
    turns: [],
    pending_proposal: null,
    resolutions: [],
    last_agent_revision: null,
    last_export: {
      revision: 4,
      status: "exported",
      attempted_at: "2026-08-01T02:00:00Z",
    },
    template_import: null,
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
  };
}

function renderCanvas({
  content,
  editing = false,
  edit,
  onChange = () => {},
  run,
  runError = null,
  canStopRun = false,
  onStopRun,
  onResumeRun,
  onKeepPartialDraft,
  onDiscardRun,
}: {
  content: ReportContent;
  editing?: boolean;
  edit?: ReportDraftEdit;
  onChange?: (edit: ReportDraftEdit) => void;
  run?: DraftRunUiState;
  runError?: string | null;
  canStopRun?: boolean;
  onStopRun?: () => void;
  onResumeRun?: (instructions?: string) => void;
  onKeepPartialDraft?: () => void;
  onDiscardRun?: () => void;
}) {
  const value = workspace(content);
  return render(
    <WritingCanvas
      workspace={value}
      edit={edit ?? draftToEdit(value.draft)}
      editing={editing}
      dirty={false}
      busy={false}
      onBeginEdit={() => {}}
      onCancelEdit={() => {}}
      hasQueuedEdits={false}
      onDiscardQueued={() => {}}
      onChange={onChange}
      onSave={() => {}}
      onExport={() => {}}
      onOpenRevisions={() => {}}
      onReference={() => {}}
      status={null}
      validationErrors={[]}
      run={run}
      runError={runError}
      canStopRun={canStopRun}
      onStopRun={onStopRun}
      onResumeRun={onResumeRun}
      onKeepPartialDraft={onKeepPartialDraft}
      onDiscardRun={onDiscardRun}
    />
  );
}

function runState(
  sections: Array<[string, SectionUiState]>,
  overrides: Partial<DraftRunUiState> = {}
): DraftRunUiState {
  return {
    ...emptyDraftRun(),
    runId: "99999999-9999-4999-8999-999999999999",
    live: true,
    total: sections.length,
    sections: new Map(sections),
    ...overrides,
  };
}

function sectionOf(id: string, section: HTMLElement | Document = document) {
  return (section as ParentNode).querySelector(`[data-section-id="${id}"]`);
}

describe("ReportDocument skipped sections", () => {
  it("renders written sections normally and the skipped template copy greyed out", () => {
    render(
      <ReportDocument content={reportContent([writtenSection, skippedSection()])} />
    );

    expect(screen.getByText("Referred for attention concerns.")).toBeTruthy();

    const skipped = screen.getByTestId("deferred-section");
    expect(skipped.textContent).toContain("Summary and Clinical Interpretation");
    expect(skipped.textContent).toContain(
      "Skipped — kept from the template for reference."
    );
    expect(skipped.textContent).toContain("Not included in the exported document.");
    expect(skipped.textContent).not.toContain("Deferred — not written yet");

    // The template copy goes through the same block renderers as real content.
    const body = screen.getByTestId("skipped-template-body");
    expect(body.className).toContain("text-gray-400");
    expect(body.className).toContain("opacity-70");
    expect(body.className).toContain("select-text");
    expect(body.textContent).toContain("Template summary paragraph.");
    expect(body.querySelector("ul li")).toBeTruthy();
    expect(body.querySelector("[data-testid='report-table']")).toBeTruthy();
  });

  it("keeps the note-only placeholder when the section has no template copy", () => {
    render(
      <ReportDocument
        content={reportContent([skippedSection({ template_blocks: null })])}
      />
    );

    const skipped = screen.getByTestId("deferred-section");
    expect(skipped.textContent).toContain(
      "Skipped — kept from the template for reference."
    );
    expect(screen.queryByTestId("skipped-template-body")).toBeNull();
  });

  it("offers no reference buttons inside a skipped section", () => {
    render(
      <ReportDocument
        content={reportContent([writtenSection, skippedSection()])}
        onReference={() => {}}
      />
    );

    expect(
      screen.getByLabelText(
        "Reference Reason for Referral, paragraph 1 in Writing chat"
      )
    ).toBeTruthy();
    expect(
      screen.queryByLabelText(
        "Reference Summary and Clinical Interpretation, paragraph 1 in Writing chat"
      )
    ).toBeNull();
    expect(
      screen.queryByLabelText(
        "Reference Summary and Clinical Interpretation, table 3 in Writing chat"
      )
    ).toBeNull();
  });
});

describe("WritingCanvas skipped sections", () => {
  it("counts skipped sections on the export status line", () => {
    renderCanvas({ content: reportContent([writtenSection, skippedSection()]) });

    expect(screen.getByRole("status").textContent).toContain(
      "· 1 skipped section was left out"
    );
  });

  it("pluralizes the export note and leaves it off when nothing is skipped", () => {
    const { unmount } = renderCanvas({
      content: reportContent([
        writtenSection,
        skippedSection(),
        skippedSection({ id: "33333333-3333-4333-8333-333333333333" }),
      ]),
    });
    expect(screen.getByRole("status").textContent).toContain(
      "· 2 skipped sections were left out"
    );
    unmount();

    renderCanvas({ content: reportContent([writtenSection]) });
    expect(screen.getByRole("status").textContent).not.toContain("skipped");
  });

  it("shows a read-only template copy and an include button while editing", () => {
    renderCanvas({
      content: reportContent([writtenSection, skippedSection()]),
      editing: true,
    });

    const skipped = screen.getByTestId("deferred-section-editing");
    expect(skipped.textContent).toContain(
      "Skipped — kept from the template for reference."
    );

    const body = screen.getByTestId("skipped-template-body");
    expect(body.textContent).toContain("Template summary paragraph.");
    // The greyed copy is reference text, not an editable field, and the
    // block-adding controls stay away until the section is included.
    expect(body.querySelector("[contenteditable]")).toBeNull();
    expect(screen.getAllByRole("button", { name: "Add paragraph" })).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Include this section" })).toBeTruthy();
  });

  it("copies the template blocks into the edit buffer when the section is included", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    renderCanvas({
      content: reportContent([writtenSection, skippedSection()]),
      editing: true,
      onChange,
    });

    await user.click(screen.getByRole("button", { name: "Include this section" }));

    expect(onChange).toHaveBeenCalledTimes(1);
    const next = onChange.mock.calls[0][0] as ReportDraftEdit;
    expect(next.sections[1].skipped).toBe(false);
    expect(next.sections[1].blocks).toEqual(templateBlocks);
    // The copy is deep, so editing the section cannot mutate the accepted draft.
    expect(next.sections[1].blocks[0]).not.toBe(templateBlocks[0]);
  });

  it("includes a skipped section with no template copy as an empty section", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    renderCanvas({
      content: reportContent([skippedSection({ template_blocks: null })]),
      editing: true,
      onChange,
    });

    expect(screen.queryByTestId("skipped-template-body")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Include this section" }));

    const next = onChange.mock.calls[0][0] as ReportDraftEdit;
    expect(next.sections[0].skipped).toBe(false);
    expect(next.sections[0].blocks).toEqual([]);
  });
});

const PENDING_SECTION_ID = "44444444-4444-4444-8444-444444444444";

const pendingSection: ReportSection = {
  id: PENDING_SECTION_ID,
  heading: "Background",
  blocks: [{ kind: "paragraph", text: "Template boilerplate." }],
  skipped: false,
};

describe("ReportDocument section states", () => {
  it("marks each section with its own edge and chip", () => {
    render(
      <ReportDocument
        content={reportContent([writtenSection, pendingSection, skippedSection()])}
        sectionStates={
          new Map<string, SectionUiState>([
            [
              WRITTEN_SECTION_ID,
              { status: "drafting", content: null, error: null },
            ],
            [PENDING_SECTION_ID, { status: "pending", content: null, error: null }],
            [SKIPPED_SECTION_ID, { status: "skipped", content: null, error: null }],
          ])
        }
      />
    );

    const drafting = sectionOf(WRITTEN_SECTION_ID);
    expect(drafting?.className).toContain("border-blue-400");
    expect(drafting?.textContent).toContain("Writing");

    const pending = sectionOf(PENDING_SECTION_ID);
    expect(pending?.className).toContain("border-gray-200");
    expect(pending?.textContent).toContain("Waiting");

    expect(sectionOf(SKIPPED_SECTION_ID)?.textContent).toContain("Skipped");
  });

  it("dims a section that is being rewritten without blanking it", () => {
    render(
      <ReportDocument
        content={reportContent([writtenSection])}
        sectionStates={
          new Map<string, SectionUiState>([
            [WRITTEN_SECTION_ID, { status: "drafting", content: null, error: null }],
          ])
        }
      />
    );

    expect(screen.getByText("Referred for attention concerns.")).toBeTruthy();
    const dimmed = sectionOf(WRITTEN_SECTION_ID)?.querySelector(".opacity-60");
    expect(dimmed?.textContent).toContain("Referred for attention concerns.");
  });

  it("shows a drafted section's chip and keeps the edge tint", () => {
    render(
      <ReportDocument
        content={reportContent([writtenSection])}
        sectionStates={
          new Map<string, SectionUiState>([
            [WRITTEN_SECTION_ID, { status: "drafted", content: null, error: null }],
          ])
        }
      />
    );

    const section = sectionOf(WRITTEN_SECTION_ID);
    expect(section?.className).toContain("border-emerald-300");
    expect(section?.textContent).toContain("Drafted");
    // The chip fades rather than disappearing, so the tint is what remains.
    expect(screen.getByText("Drafted").className).toContain("transition-opacity");
  });

  it("puts a failed section's reason under its heading", () => {
    render(
      <ReportDocument
        content={reportContent([writtenSection])}
        sectionStates={
          new Map<string, SectionUiState>([
            [
              WRITTEN_SECTION_ID,
              {
                status: "failed",
                content: null,
                error: "section_attempts_exhausted",
              },
            ],
          ])
        }
      />
    );

    expect(sectionOf(WRITTEN_SECTION_ID)?.className).toContain("border-red-400");
    expect(screen.getByText("Failed")).toBeTruthy();
    expect(screen.getByTestId("section-failure").textContent).toBe(
      "section_attempts_exhausted"
    );
  });

  it("marks an untouched section as unchanged", () => {
    render(
      <ReportDocument
        content={reportContent([writtenSection])}
        sectionStates={
          new Map<string, SectionUiState>([
            [WRITTEN_SECTION_ID, { status: "kept", content: null, error: null }],
          ])
        }
      />
    );
    expect(screen.getByText("Unchanged")).toBeTruthy();
  });

  it("leaves the document alone when no run owns it", () => {
    render(<ReportDocument content={reportContent([writtenSection])} />);
    expect(screen.queryByTestId("status-chip")).toBeNull();
    expect(sectionOf(WRITTEN_SECTION_ID)?.className).toBe("");
  });
});

describe("WritingCanvas live drafting", () => {
  it("renders landed content over the accepted draft as it arrives", () => {
    renderCanvas({
      content: reportContent([writtenSection]),
      run: runState(
        [
          [
            WRITTEN_SECTION_ID,
            {
              status: "drafted",
              content: {
                id: WRITTEN_SECTION_ID,
                heading: "Reason for Referral",
                blocks: [
                  { kind: "paragraph", text: "Written live by the drafting run." },
                ],
              },
              error: null,
            },
          ],
        ],
        { title: "Psychoeducational Evaluation", drafted: 1 }
      ),
    });

    const canvas = screen.getByTestId("accepted-report-canvas");
    expect(canvas.textContent).toContain("Written live by the drafting run.");
    expect(canvas.textContent).not.toContain("Referred for attention concerns.");
    // The run's title lands live too.
    expect(canvas.textContent).toContain("Psychoeducational Evaluation");
  });

  it("counts progress honestly and offers Stop while the run is live", async () => {
    const user = userEvent.setup();
    const onStopRun = vi.fn();
    renderCanvas({
      content: reportContent([writtenSection]),
      run: runState([], { total: 9, drafted: 4 }),
      canStopRun: true,
      onStopRun,
    });

    const bar = screen.getByRole("progressbar", {
      name: "Report sections drafted",
    });
    expect(bar.getAttribute("aria-valuenow")).toBe("4");
    expect(bar.getAttribute("aria-valuemax")).toBe("9");
    expect(bar.getAttribute("aria-valuetext")).toBe("4 of 9 drafted");

    await user.click(screen.getByRole("button", { name: "Stop run" }));
    expect(onStopRun).toHaveBeenCalledTimes(1);
  });

  it("shows no progress bar while the run is still planning", () => {
    renderCanvas({
      content: reportContent([writtenSection]),
      run: runState([], { total: null }),
      canStopRun: true,
      onStopRun: vi.fn(),
    });

    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("disables Stop once the stop is already in flight", () => {
    renderCanvas({
      content: reportContent([writtenSection]),
      run: runState([], { total: 3, drafted: 1, stopping: true }),
      canStopRun: false,
      onStopRun: vi.fn(),
    });

    const stop = screen.getByRole("button", { name: "Stopping…" });
    expect((stop as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("WritingCanvas stopped run banner", () => {
  const stopped = runState([], {
    live: false,
    total: 9,
    drafted: 4,
    outcome: "stopped" as const,
  });

  it("replaces the progress strip with what the run left behind", () => {
    renderCanvas({
      content: reportContent([writtenSection]),
      run: stopped,
      onResumeRun: vi.fn(),
      onKeepPartialDraft: vi.fn(),
      onDiscardRun: vi.fn(),
    });

    expect(screen.getByTestId("draft-run-banner").textContent).toContain(
      "Stopped — 4 of 9 sections drafted and saved. Undone sections are unchanged."
    );
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("keeps a failed run's error line above the outcome", () => {
    renderCanvas({
      content: reportContent([writtenSection]),
      run: { ...stopped, outcome: "failed" },
      runError: "The model stopped responding.",
      onResumeRun: vi.fn(),
    });

    const banner = screen.getByTestId("draft-run-banner");
    expect(banner.textContent).toContain("The model stopped responding.");
    expect(banner.textContent).toContain("Stopped — 4 of 9 sections drafted");
  });

  it("starts back up with the instructions the reader typed", async () => {
    const user = userEvent.setup();
    const onResumeRun = vi.fn();
    renderCanvas({
      content: reportContent([writtenSection]),
      run: stopped,
      onResumeRun,
    });

    await user.type(
      screen.getByLabelText("Instructions for picking the draft back up"),
      "Tighten the summary."
    );
    await user.click(screen.getByRole("button", { name: "Start back up" }));

    expect(onResumeRun).toHaveBeenCalledWith("Tighten the summary.");
  });

  it("confirms before keeping the partial draft", async () => {
    const user = userEvent.setup();
    const onKeepPartialDraft = vi.fn();
    renderCanvas({
      content: reportContent([writtenSection]),
      run: stopped,
      onKeepPartialDraft,
    });

    await user.click(screen.getByRole("button", { name: "Keep partial draft" }));
    const dialog = screen.getByRole("dialog", {
      name: "Keep the partial draft?",
    });
    expect(dialog.textContent).toContain("Undone sections will be marked skipped");
    expect(onKeepPartialDraft).not.toHaveBeenCalled();

    await user.click(
      within(dialog).getByRole("button", { name: "Keep partial draft" })
    );
    expect(onKeepPartialDraft).toHaveBeenCalledTimes(1);
  });

  it("confirms before discarding the run", async () => {
    const user = userEvent.setup();
    const onDiscardRun = vi.fn();
    renderCanvas({
      content: reportContent([writtenSection]),
      run: stopped,
      onDiscardRun,
    });

    await user.click(screen.getByRole("button", { name: "Discard run" }));
    const dialog = screen.getByRole("dialog", {
      name: "Discard this drafting run?",
    });
    expect(onDiscardRun).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Discard run" }));
    expect(onDiscardRun).toHaveBeenCalledTimes(1);
  });
});
