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
  pickTemplate: vi.fn(),
  applyTemplate: vi.fn(),
  discardTemplate: vi.fn(),
  reviewTemplate: vi.fn(),
}));

vi.mock("../lib/tauri", () => ({
  loadReportWorkspace: mocks.load,
  saveReportDraft: mocks.save,
  sendReportMessage: mocks.send,
  resolveReportProposal: mocks.resolve,
  exportReportDocx: mocks.exportDocx,
  pickReportTemplateDocx: mocks.pickTemplate,
  applyReportTemplate: mocks.applyTemplate,
  discardReportTemplatePreview: mocks.discardTemplate,
  acknowledgeReportTemplateReview: mocks.reviewTemplate,
}));

import Writing from "./Writing";

const models = [{ model_id: "model-1", name: "Claude Sonnet" }];
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
    schema_version: 2,
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
    <Writing
      clientId={clientId}
      expectedReportId={expectedReportId}
      chatModels={models}
      chatModelsLoading={false}
      chatModelsError={null}
      preferredModelId="model-1"
    />
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
  mocks.pickTemplate.mockResolvedValue(null);
  mocks.discardTemplate.mockResolvedValue(undefined);
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
      []
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
    expect(screen.getByLabelText("Referenced report paragraphs").textContent).toContain(
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
      [{ section_id: SECTION_ID, block_index: 0 }]
    );
  });

  it("shows the accepted report and record reads in Context control", async () => {
    mocks.load.mockResolvedValue(
      workspace({ assistantMarkdown: "Read it", contextReads: true })
    );
    renderWriting();
    await screen.findByText("Read it");

    await userEvent.click(screen.getByRole("button", { name: /Context/ }));
    expect(screen.getByText(/Complete accepted report · revision 0/)).toBeDefined();
    expect(screen.getByText("intake.txt")).toBeDefined();
    expect(screen.getByText(/chars 0–120/)).toBeDefined();
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

  it("lets users persistently hide both explanatory notices", async () => {
    renderWriting();
    await screen.findByText("Writing assistant");

    await userEvent.click(
      screen.getByRole("button", { name: "Hide Writing assistant notice" })
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Hide local export notice" })
    );
    expect(screen.queryByText("Writing assistant")).toBeNull();
    expect(screen.queryByText(/Local exports may contain PHI/)).toBeNull();
    expect(window.localStorage.getItem("claria.writing.hide_intro_notice")).toBe(
      "true"
    );
  });

  it("previews a DOCX import and requires carryover review before export", async () => {
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
      imported_revision: 1,
      imported_at: "2026-08-01T02:00:00Z",
      warnings: preview.warnings,
      reviewed_revision: null,
      review_required: true,
      placeholder_count: 1,
    };
    const reviewed = structuredClone(imported);
    reviewed.template_import = {
      ...imported.template_import,
      reviewed_revision: 1,
      review_required: false,
    };
    mocks.pickTemplate.mockResolvedValue(preview);
    mocks.applyTemplate.mockResolvedValue(imported);
    mocks.reviewTemplate.mockResolvedValue(reviewed);
    renderWriting();
    await screen.findByText("Accepted report title");

    await userEvent.click(screen.getByRole("button", { name: "Import .docx" }));
    expect(await screen.findByText("Review DOCX template import")).toBeDefined();
    expect(screen.getByText("Structured content preview")).toBeDefined();
    expect(screen.getByText("Headers or footers were omitted. (2)")).toBeDefined();
    const apply = screen.getByRole("button", {
      name: "Import as accepted revision",
    }) as HTMLButtonElement;
    expect(apply.disabled).toBe(true);
    await userEvent.click(
      screen.getByText(/I reviewed the structured preview/)
    );
    expect(apply.disabled).toBe(false);
    await userEvent.click(apply);

    expect(mocks.applyTemplate).toHaveBeenCalledWith("client-1", 0, "import-1");
    expect(await screen.findByText(/carryover review required/)).toBeDefined();
    const exportButton = screen.getByRole("button", {
      name: "Export .docx",
    }) as HTMLButtonElement;
    expect(exportButton.disabled).toBe(true);
    await userEvent.click(screen.getByRole("button", { name: "Mark reviewed" }));
    expect(mocks.reviewTemplate).toHaveBeenCalledWith(
      "client-1",
      "report-1",
      1
    );
    expect(exportButton.disabled).toBe(false);
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

  it("preserves a failed instruction and its paragraph references for retry", async () => {
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
    expect(screen.getByLabelText("Referenced report paragraphs")).toBeDefined();
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
      <Writing
        clientId="client-b"
        chatModels={models}
        chatModelsLoading={false}
        chatModelsError={null}
        preferredModelId="model-1"
      />
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
