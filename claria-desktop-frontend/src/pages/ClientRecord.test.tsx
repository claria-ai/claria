import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  loadReport: vi.fn(),
  sendReport: vi.fn(),
  listFiles: vi.fn(),
  listEditorHistory: vi.fn(),
  getPrompt: vi.fn(),
  listContext: vi.fn(),
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async () => () => undefined,
  }),
}));

vi.mock("../lib/tauri", () => ({
  listRecordFiles: mocks.listFiles,
  listEditorHistory: mocks.listEditorHistory,
  searchRecordContents: vi.fn().mockResolvedValue([]),
  uploadRecordFile: vi.fn(),
  deleteRecordFile: vi.fn(),
  getRecordFileText: vi.fn(),
  createTextRecordFile: vi.fn(),
  updateTextRecordFile: vi.fn(),
  loadChatHistory: vi.fn(),
  listFileVersions: vi.fn().mockResolvedValue([]),
  getFileVersionText: vi.fn(),
  restoreFileVersion: vi.fn(),
  listDeletedFiles: vi.fn().mockResolvedValue([]),
  restoreDeletedFile: vi.fn(),
  getWhisperModels: vi.fn().mockResolvedValue([]),
  transcribeMemo: vi.fn(),
  loadConfig: vi.fn().mockRejectedValue(new Error("not configured in test")),
  getPrompt: mocks.getPrompt,
  listRecordContext: mocks.listContext,
  countClientContextTokens: vi.fn(),
  extractRecordFile: vi.fn(),
  chatMessage: vi.fn(),
  lookupModelPricing: vi.fn().mockResolvedValue(null),
  acceptModelAgreement: vi.fn(),
  loadReportWorkspace: mocks.loadReport,
  saveReportDraft: vi.fn(),
  sendReportMessage: mocks.sendReport,
  resolveReportProposal: vi.fn(),
  exportReportDocx: vi.fn(),
}));

import ClientRecord from "./ClientRecord";

const workspace = {
  schema_version: 1,
  report_id: "report-1",
  client_id: "client-1",
  draft: {
    revision: 0,
    content: { title: "Accepted report", sections: [] },
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
    last_applied_proposal_id: null,
  },
  turns: [],
  pending_proposal: null,
  resolutions: [],
  last_agent_revision: null,
  last_export: null,
  created_at: "2026-08-01T00:00:00Z",
  updated_at: "2026-08-01T00:00:00Z",
};

function renderClient() {
  return render(
    <ClientRecord
      navigate={vi.fn()}
      clientId="client-1"
      clientName="Ada"
      chatModels={[{ model_id: "model-1", name: "Claude" }]}
      chatModelsLoading={false}
      chatModelsError={null}
      preferredModelId="model-1"
    />
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.listFiles.mockResolvedValue([]);
  mocks.listEditorHistory.mockResolvedValue([]);
  mocks.getPrompt.mockResolvedValue("");
  mocks.listContext.mockResolvedValue([]);
  mocks.loadReport.mockResolvedValue(workspace);
});

describe("ClientRecord Writing integration", () => {
  it("does not issue Writing IPC while Record or Chat is active", async () => {
    renderClient();
    await screen.findByText(/Drag files here/);
    expect(mocks.loadReport).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("tab", { name: "Chat" }));
    expect(await screen.findByText("Start the conversation.")).toBeDefined();
    expect(mocks.loadReport).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("tab", { name: "Writing" }));
    expect(await screen.findByTestId("accepted-report-canvas")).toBeDefined();
    expect(mocks.loadReport).toHaveBeenCalledTimes(1);
    expect(mocks.loadReport).toHaveBeenCalledWith("client-1");
  });

  it("keeps Record usable when Editor History is unavailable", async () => {
    mocks.listEditorHistory.mockRejectedValue(new Error("history unavailable"));
    renderClient();

    expect(await screen.findByText(/Drag files here/)).toBeDefined();
    expect(screen.queryByText("Error: history unavailable")).toBeNull();
  });

  it("shows Editor History and resumes its persisted Writing session", async () => {
    mocks.listEditorHistory.mockResolvedValue([
      {
        report_id: "report-1",
        title: "Psychological report",
        revision: 3,
        turn_count: 4,
        updated_at: "2026-08-01T01:00:00Z",
        last_export: {
          revision: 3,
          status: "exported",
          attempted_at: "2026-08-01T01:00:00Z",
        },
      },
    ]);
    renderClient();
    await screen.findByText("Editor History");
    await userEvent.click(screen.getByText("Editor History"));
    expect(screen.getByText("Psychological report")).toBeDefined();

    await userEvent.click(
      screen.getByRole("button", { name: "Resume writing session" })
    );
    expect(
      screen.getByRole("tab", { name: "Writing" }).getAttribute("aria-selected")
    ).toBe("true");
    expect(await screen.findByTestId("accepted-report-canvas")).toBeDefined();
  });

  it("prompts before leaving dirty inline report edits", async () => {
    renderClient();
    await userEvent.click(screen.getByRole("tab", { name: "Writing" }));
    await screen.findByTestId("accepted-report-canvas");
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    const title = screen.getByRole("textbox", { name: "Report title" });
    title.innerText = "Unsaved title";
    fireEvent.input(title);

    const confirm = vi.fn().mockReturnValue(false);
    vi.stubGlobal("confirm", confirm);
    await userEvent.click(screen.getByRole("tab", { name: "Chat" }));
    expect(confirm).toHaveBeenCalledWith(
      "Discard your unsaved Writing work, including typed instructions?"
    );
    expect(
      screen.getByRole("tab", { name: "Writing" }).getAttribute(
        "aria-selected"
      )
    ).toBe("true");
    expect(screen.getByRole("textbox", { name: "Report title" }).textContent).toBe(
      "Unsaved title"
    );

    confirm.mockReturnValue(true);
    await userEvent.click(screen.getByRole("tab", { name: "Chat" }));
    expect(
      screen.getByRole("tab", { name: "Chat" }).getAttribute("aria-selected")
    ).toBe("true");
    vi.unstubAllGlobals();
  });

  it("guards typed instructions and blocks navigation during a turn", async () => {
    mocks.sendReport.mockReturnValue(new Promise(() => undefined));
    renderClient();
    await userEvent.click(screen.getByRole("tab", { name: "Writing" }));
    await screen.findByTestId("accepted-report-canvas");
    await userEvent.type(
      screen.getByLabelText("Writing instruction"),
      "Keep this typed instruction"
    );
    const beforeUnload = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(beforeUnload);
    expect(beforeUnload.defaultPrevented).toBe(true);

    const confirm = vi.fn().mockReturnValue(false);
    vi.stubGlobal("confirm", confirm);
    await userEvent.click(screen.getByRole("tab", { name: "Record" }));
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(
      (screen.getByLabelText("Writing instruction") as HTMLTextAreaElement).value
    ).toBe("Keep this typed instruction");

    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    const alert = vi.fn();
    vi.stubGlobal("alert", alert);
    await userEvent.click(screen.getByRole("tab", { name: "Chat" }));
    expect(alert).toHaveBeenCalledWith(
      "Wait for the current Writing action to finish before leaving."
    );
    expect(
      screen.getByRole("tab", { name: "Writing" }).getAttribute(
        "aria-selected"
      )
    ).toBe("true");
    vi.unstubAllGlobals();
  });
});
