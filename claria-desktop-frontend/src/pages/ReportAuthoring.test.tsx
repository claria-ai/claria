import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReportWorkspaceView } from "../lib/tauri";

const mocks = vi.hoisted(() => ({
  load: vi.fn(),
  save: vi.fn(),
  send: vi.fn(),
  resolve: vi.fn(),
  exportDocx: vi.fn(),
}));

vi.mock("../lib/tauri", () => ({
  loadReportWorkspace: mocks.load,
  saveReportDraft: mocks.save,
  sendReportMessage: mocks.send,
  resolveReportProposal: mocks.resolve,
  exportReportDocx: mocks.exportDocx,
}));

import ReportAuthoring from "./ReportAuthoring";

const models = [{ model_id: "model-1", name: "Claude Sonnet" }];

function workspace({
  title = "Accepted report title",
  pending = false,
  revision = 0,
}: {
  title?: string;
  pending?: boolean;
  revision?: number;
} = {}): ReportWorkspaceView {
  return {
    schema_version: 1,
    report_id: "report-1",
    client_id: "client-1",
    draft: {
      revision,
      content: { title, sections: [] },
      created_at: "2026-08-01T00:00:00Z",
      updated_at: "2026-08-01T00:00:00Z",
      last_applied_proposal_id: null,
    },
    turns: [],
    pending_proposal: pending
      ? {
          id: "proposal-1",
          report_id: "report-1",
          base_revision: revision,
          model_id: "model-1",
          summary: "Rename the accepted report",
          operations: [{ kind: "set_title", title: "Proposed report title" }],
          proposed_content: {
            title: "Proposed report title",
            sections: [],
          },
          created_at: "2026-08-01T01:00:00Z",
        }
      : null,
    resolutions: [],
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
  };
}

function renderReport(clientId = "client-1") {
  return render(
    <ReportAuthoring
      clientId={clientId}
      chatModels={models}
      chatModelsLoading={false}
      chatModelsError={null}
      preferredModelId="model-1"
    />
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.load.mockResolvedValue(workspace());
  mocks.exportDocx.mockResolvedValue({
    exported: true,
    report_id: "report-1",
    revision: 0,
  });
  vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("ReportAuthoring", () => {
  it("loads lazily on mount and keeps pending proposal text out of the accepted canvas", async () => {
    mocks.load.mockResolvedValue(workspace({ pending: true }));
    renderReport();

    const canvas = within(await screen.findByTestId("accepted-report-canvas"));
    expect(mocks.load).toHaveBeenCalledWith("client-1");
    expect(canvas.getByText("Accepted report title")).toBeDefined();
    expect(canvas.queryByText("Proposed report title")).toBeNull();
    expect(screen.getByTestId("report-proposal").textContent).toContain(
      "Proposed report title"
    );
    expect(
      (screen.getByLabelText("Report instruction") as HTMLTextAreaElement)
        .disabled
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: "Edit" }) as HTMLButtonElement)
        .disabled
    ).toBe(true);
  });

  it("does not update the accepted canvas until acceptance succeeds", async () => {
    mocks.load.mockResolvedValue(workspace({ pending: true }));
    let finish!: (value: ReportWorkspaceView) => void;
    mocks.resolve.mockReturnValue(
      new Promise<ReportWorkspaceView>((resolve) => {
        finish = resolve;
      })
    );
    renderReport();
    await screen.findByTestId("report-proposal");

    await userEvent.click(
      screen.getByRole("button", { name: "Accept & save" })
    );
    const canvas = within(screen.getByTestId("accepted-report-canvas"));
    expect(canvas.getByText("Accepted report title")).toBeDefined();
    expect(canvas.queryByText("Proposed report title")).toBeNull();

    await act(async () => {
      finish(workspace({ title: "Proposed report title", revision: 1 }));
    });
    expect(
      within(screen.getByTestId("accepted-report-canvas")).getByText(
        "Proposed report title"
      )
    ).toBeDefined();
    expect(screen.queryByTestId("report-proposal")).toBeNull();
  });

  it("preserves a failed instruction for retry", async () => {
    mocks.send.mockRejectedValue(new Error("Bedrock unavailable"));
    renderReport();
    await screen.findByText("Accepted report title");

    const composer = screen.getByLabelText(
      "Report instruction"
    ) as HTMLTextAreaElement;
    await userEvent.type(composer, "Draft the findings section");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "Error: Bedrock unavailable"
    );
    expect(composer.value).toBe("Draft the findings section");
  });

  it("keeps unsaved manual edits after a save failure and offers an explicit reload", async () => {
    mocks.save.mockRejectedValue(
      new Error("The report changed on another computer. Reload it before continuing.")
    );
    renderReport();
    await screen.findByText("Accepted report title");

    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    const title = screen.getByLabelText("Report title") as HTMLInputElement;
    await userEvent.clear(title);
    await userEvent.type(title, "Locally edited title");
    await userEvent.click(screen.getByRole("button", { name: "Save Draft" }));

    expect(await screen.findByRole("alert")).toBeDefined();
    expect(title.value).toBe("Locally edited title");

    mocks.load.mockResolvedValue(
      workspace({ title: "Remote accepted title", revision: 1 })
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Reload workspace" })
    );
    expect(await screen.findByText("Remote accepted title")).toBeDefined();
    expect(mocks.load).toHaveBeenCalledTimes(2);
  });

  it("blocks invalid manual placeholders with inline validation", async () => {
    renderReport();
    await screen.findByText("Accepted report title");
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    await userEvent.clear(screen.getByLabelText("Report title"));
    expect(screen.getByRole("alert").textContent).toContain(
      "Report title must not be empty."
    );
    expect(
      (screen.getByRole("button", { name: "Save Draft" }) as HTMLButtonElement)
        .disabled
    ).toBe(true);
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it("announces canceled exports without changing the draft", async () => {
    mocks.exportDocx.mockResolvedValue({
      exported: false,
      report_id: "report-1",
      revision: 0,
    });
    renderReport();
    await screen.findByText("Accepted report title");

    await userEvent.click(
      screen.getByRole("button", { name: "Export .docx" })
    );
    expect(await screen.findByText("Export canceled.")).toBeDefined();
    expect(window.confirm).toHaveBeenCalledWith(
      "This report may contain PHI. Exporting creates an unencrypted local .docx outside Claria's managed storage. Continue?"
    );
    expect(mocks.exportDocx).toHaveBeenCalledWith(
      "client-1",
      "report-1",
      0
    );
    expect(
      within(screen.getByTestId("accepted-report-canvas")).getByText(
        "Accepted report title"
      )
    ).toBeDefined();
  });

  it("shows the complete final proposal candidate in persisted order", async () => {
    const value = workspace({ pending: true });
    value.draft.content.sections = [
      {
        id: "one",
        heading: "Current first",
        blocks: [{ kind: "paragraph", text: "Old one" }],
      },
      {
        id: "two",
        heading: "Current second",
        blocks: [{ kind: "paragraph", text: "Old two" }],
      },
    ];
    if (!value.pending_proposal) throw new Error("expected proposal");
    value.pending_proposal.proposed_content = {
      title: "Final title",
      sections: [
        {
          id: "two",
          heading: "Final first",
          blocks: [{ kind: "paragraph", text: "New two" }],
        },
        {
          id: "one",
          heading: "Final second",
          blocks: [{ kind: "paragraph", text: "New one" }],
        },
      ],
    };
    mocks.load.mockResolvedValue(value);
    renderReport();

    const candidate = within(await screen.findByTestId("proposal-final-candidate"));
    expect(candidate.getByText("Final title")).toBeDefined();
    const first = candidate.getByText("Final first");
    const second = candidate.getByText("Final second");
    expect(
      first.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING
    ).not.toBe(0);
  });

  it("distinguishes model loading and exposes an inline retry", async () => {
    const retry = vi.fn();
    const rendered = render(
      <ReportAuthoring
        clientId="client-1"
        chatModels={[]}
        chatModelsLoading
        chatModelsError={null}
        onRetryModels={retry}
      />
    );
    await screen.findByText("Accepted report title");
    expect(screen.getByRole("option", { name: "Loading models…" })).toBeDefined();

    rendered.rerender(
      <ReportAuthoring
        clientId="client-1"
        chatModels={[]}
        chatModelsLoading={false}
        chatModelsError="Network unavailable"
        onRetryModels={retry}
      />
    );
    expect(screen.getByRole("option", { name: "No models available" })).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledTimes(1);
  });

  it("suppresses a stale load after the component switches clients", async () => {
    let finishA!: (value: ReportWorkspaceView) => void;
    let finishB!: (value: ReportWorkspaceView) => void;
    mocks.load.mockImplementation((clientId: string) =>
      new Promise<ReportWorkspaceView>((resolve) => {
        if (clientId === "client-a") finishA = resolve;
        else finishB = resolve;
      })
    );
    const rendered = renderReport("client-a");
    rendered.rerender(
      <ReportAuthoring
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
