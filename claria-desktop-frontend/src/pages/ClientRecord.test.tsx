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
  getClientRecordDetails: vi.fn(),
  updateClientName: vi.fn(),
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async () => () => undefined,
  }),
}));

vi.mock("../lib/tauri", () => ({
  listRecordFiles: mocks.listFiles,
  listEditorHistory: mocks.listEditorHistory,
  getClientRecordDetails: mocks.getClientRecordDetails,
  updateClientName: mocks.updateClientName,
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
  getLocalTranscriptionStatus: vi.fn().mockResolvedValue({
    accelerated: false,
    models: [],
  }),
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
  pickReportTemplateDocx: vi.fn().mockResolvedValue(null),
  applyReportTemplate: vi.fn(),
  discardReportTemplatePreview: vi.fn(),
  acknowledgeReportTemplateReview: vi.fn(),
}));

import { clearWritingComposerDrafts } from "../lib/writingComposerDraft";
import ClientRecord from "./ClientRecord";

const workspace = {
  schema_version: 2,
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
  template_import: null,
  created_at: "2026-08-01T00:00:00Z",
  updated_at: "2026-08-01T00:00:00Z",
};

function renderClient(navigate = vi.fn(), onNameChanged = vi.fn()) {
  return render(
    <ClientRecord
      navigate={navigate}
      clientId="client-1"
      clientName="Ada"
      chatModels={[{ model_id: "model-1", name: "Claude" }]}
      chatModelsLoading={false}
      chatModelsError={null}
      preferredModelId="model-1"
      onClientNameChanged={onNameChanged}
    />
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  clearWritingComposerDrafts();
  mocks.listFiles.mockResolvedValue([]);
  mocks.listEditorHistory.mockResolvedValue([]);
  mocks.getPrompt.mockResolvedValue("");
  mocks.listContext.mockResolvedValue([]);
  mocks.loadReport.mockResolvedValue(workspace);
  mocks.getClientRecordDetails.mockResolvedValue({
    id: "client-1",
    name: "Ada",
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
    file_count: 3,
    storage_bytes: 1_572_864,
    storage_bytes_with_history: 2_097_152,
    name_history: [
      { name: "Ada", changed_at: "2026-08-01T00:00:00Z" },
      {
        name: "Augusta Ada King",
        changed_at: "2026-07-01T00:00:00Z",
      },
    ],
  });
  mocks.updateClientName.mockResolvedValue({
    id: "client-1",
    name: "Ada",
    updated_at: "2026-08-01T00:00:00Z",
  });
});

describe("ClientRecord Writing integration", () => {
  it("loads record settings only after the gear is opened", async () => {
    renderClient();
    await screen.findByText(/Drag files here/);
    expect(mocks.getClientRecordDetails).not.toHaveBeenCalled();

    await userEvent.click(
      screen.getByRole("button", { name: "Record settings" })
    );
    expect(await screen.findByText("Record settings")).toBeDefined();
    expect(mocks.getClientRecordDetails).toHaveBeenCalledWith("client-1");
    expect(screen.getByText("3 files")).toBeDefined();
    expect(screen.getByText("1.5 MB")).toBeDefined();
    const historicalStorage = screen.getByText("Including history: 2.0 MB");
    expect(historicalStorage.className).toContain("text-gray-400");
    expect(screen.queryByText("Record details")).toBeNull();
    expect(screen.queryByText("Record statistics")).toBeNull();
    expect(screen.getByRole("table", { name: "Record name history" })).toBeDefined();
    expect(screen.getByRole("columnheader", { name: "Name" })).toBeDefined();
    expect(screen.getByRole("columnheader", { name: "Time" })).toBeDefined();
    expect(screen.getByRole("cell", { name: "Augusta Ada King" })).toBeDefined();
  });

  it("renames a record from settings", async () => {
    const onNameChanged = vi.fn();
    mocks.updateClientName.mockResolvedValue({
      id: "client-1",
      name: "Ada Byron",
      updated_at: "2026-08-02T00:00:00Z",
    });
    renderClient(vi.fn(), onNameChanged);
    await userEvent.click(
      screen.getByRole("button", { name: "Record settings" })
    );
    const input = await screen.findByLabelText("Record name");
    await userEvent.clear(input);
    await userEvent.type(input, "Ada Byron");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(mocks.updateClientName).toHaveBeenCalledWith(
      "client-1",
      "Ada Byron"
    );
    expect(onNameChanged).toHaveBeenCalledWith("Ada Byron");
    expect(await screen.findByText("Record name updated.")).toBeDefined();
    expect(
      screen.getByRole("cell", { name: "Ada Byron" })
    ).toBeDefined();
  });

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
      "Discard unsaved report edits? Your typed Writing instruction and references will be kept."
    );
    expect(
      screen.getByRole("tab", { name: "Writing" }).getAttribute(
        "aria-selected"
      )
    ).toBe("true");
    expect(screen.getByRole("textbox", { name: "Report title" }).textContent).toBe(
      "Unsaved title"
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Record settings" })
    );
    expect(screen.queryByText("Record settings")).toBeNull();
    expect(mocks.getClientRecordDetails).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    await userEvent.click(screen.getByRole("tab", { name: "Chat" }));
    expect(
      screen.getByRole("tab", { name: "Chat" }).getAttribute("aria-selected")
    ).toBe("true");
    vi.unstubAllGlobals();
  });

  it("allows the back caret when only a Writing instruction is typed", async () => {
    const navigate = vi.fn();
    const confirm = vi.fn().mockReturnValue(false);
    vi.stubGlobal("confirm", confirm);
    renderClient(navigate);
    await userEvent.click(screen.getByRole("tab", { name: "Writing" }));
    await screen.findByTestId("accepted-report-canvas");
    await userEvent.type(
      screen.getByLabelText("Writing instruction"),
      "Keep this typed instruction"
    );

    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(navigate).toHaveBeenCalledWith("clients");
    expect(confirm).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });

  it("retains typed instructions across tabs and blocks navigation during a turn", async () => {
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
    expect(confirm).not.toHaveBeenCalled();
    expect(
      screen.getByRole("tab", { name: "Record" }).getAttribute("aria-selected")
    ).toBe("true");

    await userEvent.click(screen.getByRole("tab", { name: "Writing" }));
    await screen.findByTestId("accepted-report-canvas");
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
