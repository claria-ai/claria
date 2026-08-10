import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReportWorkspaceView } from "../lib/tauri";
import { clearWritingComposerDrafts } from "../lib/writingComposerDraft";

const mocks = vi.hoisted(() => ({
  load: vi.fn(),
  save: vi.fn(),
  send: vi.fn(),
  resolve: vi.fn(),
  exportDocx: vi.fn(),
  listTemplates: vi.fn(),
  previewTemplate: vi.fn(),
  applyTemplate: vi.fn(),
  discardTemplate: vi.fn(),
  getRecordText: vi.fn(),
  listRevisions: vi.fn(),
  loadRevision: vi.fn(),
  revertRevision: vi.fn(),
}));

vi.mock("../lib/tauri", () => ({
  // The ledger's pricing map resolves per-model pricing; `null` keeps the
  // cost-explanation rows unpriced without hitting the bridge.
  lookupModelPricing: vi.fn(async () => null),
  loadReportWorkspace: mocks.load,
  saveReportDraft: mocks.save,
  sendReportMessage: mocks.send,
  resolveReportProposal: mocks.resolve,
  exportReportDocx: mocks.exportDocx,
  listWriterTemplates: mocks.listTemplates,
  previewWriterTemplate: mocks.previewTemplate,
  applyReportTemplate: mocks.applyTemplate,
  discardReportTemplatePreview: mocks.discardTemplate,
  getRecordFileText: mocks.getRecordText,
  listReportRevisions: mocks.listRevisions,
  loadReportRevision: mocks.loadRevision,
  revertReportRevision: mocks.revertRevision,
}));

import Writing from "./Writing";
import { ChatModelsContext, type ChatModelsState } from "../lib/chatModels";

const models = [{ model_id: "model-1", name: "Claude Sonnet" }];
const modelsState: ChatModelsState = {
  models,
  loading: false,
  error: null,
  preferredModelId: "model-1",
  retry: () => {},
  setPreferredModelId: () => {},
};
const SECTION_ID = "11111111-1111-4111-8111-111111111111";

function workspace({
  title = "Accepted report title",
  paragraph = "A **bold report** paragraph.",
  pending = false,
  revision = 0,
  assistantMarkdown,
  contextReads = false,
  lastAgentRevision = null,
}: {
  title?: string;
  paragraph?: string;
  pending?: boolean;
  revision?: number;
  assistantMarkdown?: string;
  contextReads?: boolean;
  lastAgentRevision?: number | null;
} = {}): ReportWorkspaceView {
  return {
    schema_version: 4,
    session_name: "Writer Session (1)",
    report_id: "report-1",
    client_id: "client-1",
    draft: {
      revision,
      content: {
        title,
        sections: [
          {
            id: SECTION_ID,
            heading: "Findings",
            blocks: [{ kind: "paragraph", text: paragraph }],
          },
        ],
      },
      created_at: "2026-08-01T00:00:00Z",
      updated_at: "2026-08-01T00:00:00Z",
      last_applied_proposal_id: null,
    },
    turns: assistantMarkdown
      ? [
          {
            id: "turn-1",
            model_id: "model-1",
            timeline: [
              {
                kind: "message",
                role: "assistant",
                text: assistantMarkdown,
                created_at: "2026-08-01T00:01:00Z",
              },
            ],
            usage: {
              model_id: "model-1",
              input_tokens: 10,
              output_tokens: 5,
              cache_read_input_tokens: 0,
              cache_write_input_tokens: 0,
              cost_usd: 0.001,
              pricing_version: 1,
            },
            usage_complete: true,
            converse_calls: 1,
            tool_uses: contextReads ? 1 : 0,
            context_reads: contextReads
              ? [
                  {
                    filename: "intake.txt",
                    offset: 0,
                    returned_characters: 120,
                    total_characters: 500,
                    read_at: "2026-08-01T00:01:00Z",
                  },
                ]
              : [],
            created_at: "2026-08-01T00:01:00Z",
            completed_at: "2026-08-01T00:01:01Z",
          },
        ]
      : [],
    pending_proposal: pending
      ? {
          id: "proposal-1",
          report_id: "report-1",
          base_revision: revision,
          model_id: "model-1",
          summary: "Rename the **accepted** report",
          operations: [{ kind: "set_title", title: "Proposed report title" }],
          proposed_content: {
            title: "Proposed report title",
            sections: [
              {
                id: SECTION_ID,
                heading: "Findings",
                blocks: [{ kind: "paragraph", text: paragraph }],
              },
            ],
          },
          created_at: "2026-08-01T01:00:00Z",
        }
      : null,
    resolutions: [],
    last_agent_revision: lastAgentRevision,
    last_export: null,
    template_import: null,
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
  };
}

function turnResponse(value: ReportWorkspaceView) {
  return {
    workspace: value,
    turn_id: "turn-new",
    attempt_id: "attempt-1",
    assistant_text: "Done",
    usage: {
      model_id: "model-1",
      input_tokens: 10,
      output_tokens: 5,
      cache_read_input_tokens: 0,
      cache_write_input_tokens: 0,
      cost_usd: 0.001,
      pricing_version: 1,
    },
    usage_complete: true,
    converse_calls: 1,
    tool_uses: 0,
    proposal_id: null,
  };
}

function renderWriting(clientId = "client-1", expectedReportId?: string) {
  return render(
    <ChatModelsContext.Provider value={modelsState}>
      <Writing
        clientId={clientId}
        expectedReportId={expectedReportId}
        onManageTemplates={vi.fn()}
      />
    </ChatModelsContext.Provider>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  clearWritingComposerDrafts();
  const stored = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => stored.set(key, value),
    removeItem: (key: string) => stored.delete(key),
    clear: () => stored.clear(),
  });
  mocks.load.mockResolvedValue(workspace());
  mocks.listTemplates.mockResolvedValue([]);
  mocks.save.mockImplementation(
    (_clientId: string, revision: number, draft: { title: string; sections: ReportWorkspaceView["draft"]["content"]["sections"] }) => {
      const saved = workspace({ title: draft.title, revision: revision + 1 });
      saved.draft.content.sections = draft.sections.map((section) => ({
        id: section.id ?? SECTION_ID,
        heading: section.heading,
        blocks: section.blocks,
      }));
      return Promise.resolve(saved);
    }
  );
  mocks.send.mockResolvedValue(turnResponse(workspace({ lastAgentRevision: 0 })));
  mocks.previewTemplate.mockResolvedValue(null);
  mocks.discardTemplate.mockResolvedValue(undefined);
  mocks.getRecordText.mockResolvedValue("Preview text");
  mocks.listRevisions.mockResolvedValue([]);
  mocks.exportDocx.mockResolvedValue({
    exported: true,
    report_id: "report-1",
    revision: 0,
    status: "exported",
    attempted_at: "2026-08-01T02:00:00Z",
    status_persisted: true,
  });
  vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Writing", () => {
  it("renders assistant and report Markdown instead of showing markers", async () => {
    mocks.load.mockResolvedValue(
      workspace({ assistantMarkdown: "A **bold assistant** response." })
    );
    renderWriting();

    const assistant = await screen.findByText("bold assistant");
    const report = screen.getByText("bold report");
    expect(assistant.tagName).toBe("STRONG");
    expect(report.tagName).toBe("STRONG");
  });

  it("keeps a pending proposal separate from the accepted report", async () => {
    mocks.load.mockResolvedValue(workspace({ pending: true }));
    renderWriting();

    const canvas = within(await screen.findByTestId("accepted-report-canvas"));
    expect(canvas.getByText("Accepted report title")).toBeDefined();
    expect(canvas.queryByText("Proposed report title")).toBeNull();
    expect(screen.getByTestId("report-proposal").textContent).toContain(
      "Proposed report title"
    );
    expect(
      (screen.getByLabelText("Writing instruction") as HTMLTextAreaElement)
        .disabled
    ).toBe(true);
  });

  it("shows only the changed paragraph instead of repeating the complete report", async () => {
    const value = workspace({ pending: true });
    const unchangedOpening = { kind: "paragraph" as const, text: "Unchanged opening" };
    const unchangedClosing = { kind: "paragraph" as const, text: "Unchanged closing" };
    value.draft.content.sections[0].blocks = [
      unchangedOpening,
      { kind: "paragraph", text: "Old focused paragraph" },
      unchangedClosing,
    ];
    value.pending_proposal = {
      ...value.pending_proposal!,
      summary: "Revise one paragraph",
      operations: [
        {
          kind: "replace_section",
          section_id: SECTION_ID,
          heading: "Findings",
          blocks: [
            unchangedOpening,
            { kind: "paragraph", text: "New focused paragraph" },
            unchangedClosing,
          ],
        },
      ],
      proposed_content: {
        title: value.draft.content.title,
        sections: [
          {
            id: SECTION_ID,
            heading: "Findings",
            blocks: [
              unchangedOpening,
              { kind: "paragraph", text: "New focused paragraph" },
              unchangedClosing,
            ],
          },
        ],
      },
    };
    mocks.load.mockResolvedValue(value);
    renderWriting();

    const proposal = within(await screen.findByTestId("report-proposal"));
    expect(proposal.queryByText("Complete accepted vs final report")).toBeNull();
    expect(proposal.getByText("Change paragraph 2")).toBeDefined();
    expect(proposal.getByText("Old focused paragraph")).toBeDefined();
    expect(proposal.getByText("New focused paragraph")).toBeDefined();
    expect(proposal.queryByText("Unchanged opening")).toBeNull();
    expect(proposal.queryByText("Unchanged closing")).toBeNull();
  });

  it("renders and directly edits structured report tables", async () => {
    const value = workspace();
    value.draft.content.sections[0].blocks = [
      {
        kind: "table",
        rows: [
          ["Measure", "Score"],
          ["Attention", "87"],
        ],
        has_header: true,
        column_widths: [7000, 3000],
      },
    ];
    mocks.load.mockResolvedValue(value);
    renderWriting();

    const table = await screen.findByTestId("report-table");
    expect(table.textContent).toContain("Attention");
    expect(table.querySelectorAll("th")).toHaveLength(2);

    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    const score = screen.getByRole("textbox", {
      name: "Section 1 table 1, row 2, column 2",
    });
    score.innerText = "91";
    fireEvent.input(score);
    await userEvent.click(screen.getByRole("button", { name: "Add row" }));
    await userEvent.click(screen.getByRole("button", { name: "Save now" }));

    expect(mocks.save).toHaveBeenCalledWith(
      "client-1",
      0,
      expect.objectContaining({
        sections: [
          expect.objectContaining({
            blocks: [
              expect.objectContaining({
                kind: "table",
                rows: [
                  ["Measure", "Score"],
                  ["Attention", "91"],
                  ["", ""],
                ],
              }),
            ],
          }),
        ],
      })
    );
  });

  it("highlights focused cell changes in table proposals", async () => {
    const value = workspace({ pending: true });
    const currentTable = {
      kind: "table" as const,
      rows: [
        ["Measure", "Score"],
        ["Attention", "87"],
      ],
      has_header: true,
      column_widths: [7000, 3000],
    };
    const proposedTable = {
      ...currentTable,
      rows: [
        ["Measure", "Score"],
        ["Attention", "91"],
      ],
    };
    value.draft.content.sections[0].blocks = [currentTable];
    value.pending_proposal = {
      ...value.pending_proposal!,
      summary: "Update one score",
      operations: [
        {
          kind: "replace_section",
          section_id: SECTION_ID,
          heading: "Findings",
          blocks: [proposedTable],
        },
      ],
      proposed_content: {
        title: value.draft.content.title,
        sections: [
          {
            id: SECTION_ID,
            heading: "Findings",
            blocks: [proposedTable],
          },
        ],
      },
    };
    mocks.load.mockResolvedValue(value);
    renderWriting();

    const proposal = within(await screen.findByTestId("report-proposal"));
    expect(proposal.getByText("Change table 1 · 1 cell")).toBeDefined();
    expect(proposal.getByText("87").className).toContain("bg-red-50");
    expect(proposal.getByText("91").className).toContain("bg-violet-100");
  });

  it("uses transparent inline editing and saves edits before the next message", async () => {
    const saved = workspace({ title: "Edited directly", revision: 1 });
    mocks.save.mockResolvedValue(saved);
    mocks.send.mockResolvedValue(
      turnResponse(workspace({ title: "Edited directly", revision: 1, lastAgentRevision: 1 }))
    );
    renderWriting();
    await screen.findByText("Accepted report title");

    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(screen.getByTestId("inline-report-editor")).toBeDefined();
    const title = screen.getByRole("textbox", { name: "Report title" });
    title.innerText = "Edited directly";
    fireEvent.input(title);

    expect(screen.getByTestId("queued-report-edits").textContent).toContain(
      "queued"
    );
    await userEvent.type(
      screen.getByLabelText("Writing instruction"),
      "Review my edits"
    );
    await userEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(mocks.save).toHaveBeenCalledWith(
      "client-1",
      0,
      expect.objectContaining({ title: "Edited directly" })
    );
    expect(mocks.send).toHaveBeenCalledWith(
      "client-1",
      1,
      "model-1",
      "Review my edits",
      [],
      expect.any(Function)
    );
  });

  it("drops a hovered paragraph reference into the composer and sends it", async () => {
    renderWriting();
    await screen.findByText("bold report");

    await userEvent.click(
      screen.getByRole("button", {
        name: "Reference Findings, paragraph 1 in Writing chat",
      })
    );
    expect(screen.getByLabelText("Referenced report blocks").textContent).toContain(
      "Findings ¶1"
    );
    await userEvent.type(
      screen.getByLabelText("Writing instruction"),
      "Shorten this"
    );
    await userEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(mocks.send).toHaveBeenCalledWith(
      "client-1",
      0,
      "model-1",
      "Shorten this",
      [{ section_id: SECTION_ID, block_index: 0 }],
      expect.any(Function)
    );
  });

  it("attaches a hovered table to the next Writing message", async () => {
    const value = workspace();
    value.draft.content.sections[0].blocks = [
      {
        kind: "table",
        rows: [
          ["Measure", "Score"],
          ["Attention", "87"],
        ],
        has_header: true,
        column_widths: [7000, 3000],
      },
    ];
    mocks.load.mockResolvedValue(value);
    renderWriting();
    await screen.findByTestId("report-table");

    await userEvent.click(
      screen.getByRole("button", {
        name: "Reference Findings, table 1 in Writing chat",
      })
    );
    expect(screen.getByLabelText("Referenced report blocks").textContent).toContain(
      "Findings table 1: Measure | Score · Attention | 87"
    );
    await userEvent.type(
      screen.getByLabelText("Writing instruction"),
      "Update this score table"
    );
    await userEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(mocks.send).toHaveBeenCalledWith(
      "client-1",
      0,
      "model-1",
      "Update this score table",
      [{ section_id: SECTION_ID, block_index: 0 }],
      expect.any(Function)
    );
  });

  it("shows live agent activity and a context pill while a record is read", async () => {
    let finish!: (value: ReturnType<typeof turnResponse>) => void;
    mocks.send.mockImplementation(
      (
        _clientId: string,
        _revision: number,
        _modelId: string,
        _instruction: string,
        _references: unknown[],
        onProgress: (progress: unknown) => void
      ) => {
        onProgress({
          kind: "tool_started",
          name: "read_record_file",
          context: "intake.txt",
        });
        return new Promise((resolve) => {
          finish = resolve;
        });
      }
    );
    renderWriting();
    await screen.findByText("Accepted report title");
    await userEvent.type(screen.getByLabelText("Writing instruction"), "Use the intake");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(
      screen
        .getAllByRole("status")
        .some((status) => status.textContent?.includes("Reading client context"))
    ).toBe(true);
    expect(screen.queryByLabelText("Writer context")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: /Context/ }));
    expect(screen.getByLabelText("Writer context").textContent).toContain("intake.txt");

    await act(async () => {
      finish(turnResponse(workspace({ lastAgentRevision: 0 })));
    });
    expect(screen.queryByText("Reading client context")).toBeNull();
  });

  it("combines context pills in one collapsed control and previews record content", async () => {
    mocks.load.mockResolvedValue(
      workspace({ assistantMarkdown: "Read it", contextReads: true })
    );
    renderWriting();
    await screen.findByText("Read it");

    expect(screen.queryByLabelText("Writer context")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: /Context/ }));
    expect(screen.getByText("Accepted report · r0")).toBeDefined();
    expect(screen.getAllByText("intake.txt")).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "intake.txt" }));
    expect(await screen.findByText("Preview text")).toBeDefined();
    expect(mocks.getRecordText).toHaveBeenCalledWith("client-1", "intake.txt");
  });

  it("previews a previous report revision and restores it as a new revision", async () => {
    const current = workspace({ title: "Current report", revision: 2 });
    const restored = workspace({ title: "Earlier report", revision: 3 });
    mocks.load.mockResolvedValue(current);
    mocks.listRevisions.mockResolvedValue([
      {
        revision: 2,
        title: "Current report",
        updated_at: "2026-08-01T02:00:00Z",
      },
      {
        revision: 1,
        title: "Earlier report",
        updated_at: "2026-08-01T01:00:00Z",
      },
      {
        revision: 0,
        title: "Blank report",
        updated_at: "2026-08-01T00:00:00Z",
      },
    ]);
    mocks.loadRevision.mockResolvedValue({
      revision: 1,
      content: restored.draft.content,
      created_at: "2026-08-01T00:00:00Z",
      updated_at: "2026-08-01T01:00:00Z",
      last_applied_proposal_id: null,
    });
    mocks.revertRevision.mockResolvedValue(restored);
    renderWriting();
    await screen.findByText("Current report");

    await userEvent.click(screen.getByRole("button", { name: "Revisions" }));
    const historicalCanvas = await screen.findByTestId("revision-report-canvas");
    expect(within(historicalCanvas).getByText("Earlier report")).toBeDefined();
    expect(historicalCanvas.parentElement?.parentElement?.className).toContain(
      "overflow-y-auto"
    );
    expect(
      screen.queryByRole("option", { name: /Revision 2/ })
    ).toBeNull();

    await userEvent.click(
      screen.getByRole("button", { name: "Revert to version" })
    );
    expect(mocks.revertRevision).toHaveBeenCalledWith(
      "client-1",
      "report-1",
      2,
      1
    );
    expect(await screen.findByText("Restored as revision 3.")).toBeDefined();
    expect(screen.queryByTestId("revision-report-canvas")).toBeNull();
    expect(
      within(screen.getByTestId("accepted-report-canvas")).getByText(
        "Earlier report"
      )
    ).toBeDefined();
  });

  it("keeps raw LLM tool invocations nerdy and collapsed by default", async () => {
    const value = workspace({ assistantMarkdown: "Done" });
    value.turns[0].timeline.unshift({
      kind: "tool_activity",
      name: "read_record_file",
      summary: "Read intake.txt, characters 0–120",
      status: "succeeded",
      invocation_json: JSON.stringify(
        {
          toolUse: {
            toolUseId: "read-1",
            name: "read_record_file",
            input: { filename: "intake.txt", offset: 0, limit: 8000 },
          },
        },
        null,
        2
      ),
      result_json: JSON.stringify(
        {
          toolResult: {
            toolUseId: "read-1",
            status: "success",
            content: [{ json: { returned_characters: 120 } }],
          },
        },
        null,
        2
      ),
      created_at: "2026-08-01T00:01:00Z",
    });
    mocks.load.mockResolvedValue(value);
    renderWriting();

    const summary = await screen.findByText("Read intake.txt, characters 0–120");
    const details = summary.closest("details") as HTMLDetailsElement;
    expect(details.open).toBe(false);
    await userEvent.click(details.querySelector("summary")!);
    expect(details.open).toBe(true);
    expect(within(details).getByText("Raw LLM invocation")).toBeDefined();
    expect(details.textContent).toContain('"filename": "intake.txt"');
    expect(details.textContent).toContain("Correlated tool result");
  });

  it("does not show content-responsibility notices or confirmation controls", async () => {
    renderWriting();
    await screen.findByText("Accepted report title");

    expect(screen.queryByText("Writing assistant")).toBeNull();
    expect(screen.queryByText(/Local exports may contain PHI/)).toBeNull();
    expect(screen.queryByRole("button", { name: /Hide .* notice/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Mark reviewed" })).toBeNull();
  });

  it("applies a managed writer template directly without review nags", async () => {
    const preview = {
      import_id: "import-1",
      content: {
        title: "Imported evaluation",
        sections: [
          {
            id: SECTION_ID,
            heading: "Scores",
            blocks: [
              {
                kind: "table" as const,
                rows: [
                  ["Measure", "Score"],
                  ["Attention", "{{score}}"],
                ],
                has_header: true,
                column_widths: [7000, 3000],
              },
            ],
          },
        ],
      },
      warnings: [
        {
          code: "headers_footers_omitted",
          message: "Headers or footers were omitted.",
          count: 2,
        },
      ],
      stats: {
        sections: 1,
        paragraphs: 0,
        bullet_lists: 0,
        tables: 1,
        table_cells: 4,
        placeholder_count: 1,
      },
    };
    const imported = workspace({ title: "Imported evaluation", revision: 1 });
    imported.draft.content = preview.content;
    imported.template_import = {
      writer_template_id: "template-1",
      writer_template_name: "Assessment template",
      imported_revision: 1,
      imported_at: "2026-08-01T02:00:00Z",
      warnings: preview.warnings,
      reviewed_revision: 1,
      review_required: false,
      placeholder_count: 1,
    };
    mocks.listTemplates.mockResolvedValue([
      {
        id: "template-1",
        name: "Assessment template",
        size: 4096,
        uploaded_at: "2026-08-01T00:00:00Z",
        use_count: 2,
      },
    ]);
    mocks.previewTemplate.mockResolvedValue(preview);
    mocks.applyTemplate.mockResolvedValue(imported);
    renderWriting();
    await screen.findByText("Accepted report title");

    expect(await screen.findByRole("option", { name: "Assessment template" })).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Apply template" }));
    expect(mocks.previewTemplate).toHaveBeenCalledWith("client-1", "template-1");
    expect(mocks.applyTemplate).toHaveBeenCalledWith("client-1", 0, "import-1");
    expect(await screen.findByText("Imported evaluation")).toBeDefined();
    expect(screen.getByText(/Template/).textContent).toContain(
      "Template Assessment template applied"
    );
    expect(screen.queryByLabelText("Writer template")).toBeNull();
    expect(screen.queryByRole("button", { name: "Apply template" })).toBeNull();
    expect(screen.queryByText("Review DOCX template import")).toBeNull();
    expect(screen.queryByText(/carryover review required/)).toBeNull();
    expect(
      (screen.getByRole("button", { name: "Export .docx" }) as HTMLButtonElement)
        .disabled
    ).toBe(false);
  });

  it("can retry export immediately after the native dialog is canceled", async () => {
    mocks.exportDocx
      .mockResolvedValueOnce({
        exported: false,
        report_id: "report-1",
        revision: 0,
        status: "canceled",
        attempted_at: "2026-08-01T02:00:00Z",
        status_persisted: true,
      })
      .mockResolvedValueOnce({
        exported: true,
        report_id: "report-1",
        revision: 0,
        status: "exported",
        attempted_at: "2026-08-01T02:01:00Z",
        status_persisted: true,
      });
    renderWriting();
    await screen.findByText("Accepted report title");

    await userEvent.click(screen.getByRole("button", { name: "Export .docx" }));
    expect(await screen.findByText(/Export canceled. You can try again/)).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Export .docx" }));
    expect(await screen.findByText(/Word document exported from revision 0/)).toBeDefined();
    expect(mocks.exportDocx).toHaveBeenCalledTimes(2);
    expect(window.confirm).not.toHaveBeenCalled();
  });

  it("preserves a failed instruction and its report references for retry", async () => {
    mocks.send.mockRejectedValue(new Error("Bedrock unavailable"));
    renderWriting();
    await screen.findByText("bold report");
    await userEvent.click(
      screen.getByRole("button", {
        name: "Reference Findings, paragraph 1 in Writing chat",
      })
    );

    const composer = screen.getByLabelText(
      "Writing instruction"
    ) as HTMLTextAreaElement;
    await userEvent.type(composer, "Draft the findings section");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "Error: Bedrock unavailable"
    );
    expect(composer.value).toBe("Draft the findings section");
    expect(screen.getByLabelText("Referenced report blocks")).toBeDefined();
  });

  it("rejects an Editor History entry for a different report", async () => {
    renderWriting("client-1", "another-report");
    expect(
      await screen.findByText("Could not load the writing session")
    ).toBeDefined();
    expect(screen.getByRole("alert").textContent).toContain(
      "Editor History session is no longer available"
    );
  });

  it("suppresses a stale load after the component switches clients", async () => {
    let finishA!: (value: ReportWorkspaceView) => void;
    let finishB!: (value: ReportWorkspaceView) => void;
    mocks.load.mockImplementation(
      (clientId: string) =>
        new Promise<ReportWorkspaceView>((resolve) => {
          if (clientId === "client-a") finishA = resolve;
          else finishB = resolve;
        })
    );
    const rendered = renderWriting("client-a");
    rendered.rerender(
      <ChatModelsContext.Provider value={modelsState}>
        <Writing clientId="client-b" onManageTemplates={vi.fn()} />
      </ChatModelsContext.Provider>
    );

    await act(async () => {
      finishB(workspace({ title: "Client B report" }));
    });
    expect(screen.getByText("Client B report")).toBeDefined();

    await act(async () => {
      finishA(workspace({ title: "Stale client A report" }));
    });
    expect(screen.getByText("Client B report")).toBeDefined();
    expect(screen.queryByText("Stale client A report")).toBeNull();
  });
});
