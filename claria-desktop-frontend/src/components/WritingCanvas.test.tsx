import { render, screen } from "@testing-library/react";
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
}: {
  content: ReportContent;
  editing?: boolean;
  edit?: ReportDraftEdit;
  onChange?: (edit: ReportDraftEdit) => void;
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
    />
  );
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
