import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  DraftRun,
  FullReportGenerationResponse,
  ReportTurnProgressView,
  ReportWorkspaceView,
} from "./tauri";

const mocks = vi.hoisted(() => ({
  load: vi.fn(),
  generate: vi.fn(),
  loadDraftRun: vi.fn(),
  resumeDraftRun: vi.fn(),
  finalizePartialDraft: vi.fn(),
  abandonDraftRun: vi.fn(),
  stopStream: vi.fn(),
  logFrontendEvent: vi.fn(),
  loadConfig: vi.fn(),
  generateDraftPlan: vi.fn(),
  updateDraftPlan: vi.fn(),
  startDraftRun: vi.fn(),
}));

vi.mock("./logBridge", () => ({ logFrontendEvent: mocks.logFrontendEvent }));

vi.mock("./tauri", () => ({
  startReportWorkspace: mocks.load,
  loadReportWorkspace: mocks.load,
  generateFullReport: mocks.generate,
  loadDraftRun: mocks.loadDraftRun,
  resumeDraftRun: mocks.resumeDraftRun,
  loadConfig: mocks.loadConfig,
  generateDraftPlan: mocks.generateDraftPlan,
  updateDraftPlan: mocks.updateDraftPlan,
  startDraftRun: mocks.startDraftRun,
  finalizePartialDraft: mocks.finalizePartialDraft,
  abandonDraftRun: mocks.abandonDraftRun,
  stopStream: mocks.stopStream,
  saveReportDraft: vi.fn(),
  discardQueuedReportEdits: vi.fn(),
  sendReportMessage: vi.fn(),
  resolveReportProposal: vi.fn(),
  exportReportDocx: vi.fn(),
  applyReportTemplate: vi.fn(),
  previewWriterTemplate: vi.fn(),
  discardReportTemplatePreview: vi.fn(),
  renameReportSession: vi.fn(),
}));

import { useReportWorkspace } from "./useReportWorkspace";

const CLIENT_ID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const REPORT_ID = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const RUN_ID = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
const SECTION_ID = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

function workspace(): ReportWorkspaceView {
  return {
    schema_version: 7,
    report_id: REPORT_ID,
    client_id: CLIENT_ID,
    session_name: "Writer Session (1)",
    draft: {
      revision: 1,
      content: {
        title: "Evaluation",
        sections: [
          {
            id: SECTION_ID,
            heading: "Reason for Referral",
            blocks: [{ kind: "paragraph", text: "Template boilerplate." }],
            skipped: false,
            template_blocks: null,
            authorship: null,
          },
        ],
      },
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
      last_applied_proposal_id: null,
    },
    turns: [],
    pending_proposal: null,
    resolutions: [],
    last_agent_revision: null,
    last_export: null,
    template_import: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
}

function generationResponse(): FullReportGenerationResponse {
  return {
    workspace: workspace(),
    turn_id: "turn-1",
    attempt_id: "attempt-1",
    assistant_text: "Done.",
    usage: {
      model_id: "model-1",
      input_tokens: 1,
      output_tokens: 1,
      cache_read_input_tokens: 0,
      cache_write_input_tokens: 0,
      cache_ttl: null,
      cost_usd: 0,
      pricing_version: 1,
    },
    usage_complete: true,
    converse_calls: 1,
    tool_uses: 1,
    included_record_files: 1,
    unavailable_record_files: 0,
    record_characters: 10,
  };
}

function stoppedRun(): DraftRun {
  return {
    schema_version: 1,
    run_id: RUN_ID,
    report_id: REPORT_ID,
    client_id: CLIENT_ID,
    base_revision: 1,
    status: "stopped",
    plan: null,
    title: "Psychoeducational Evaluation",
    sections: [
      {
        section_id: SECTION_ID,
        heading: "Reason for Referral",
        position: 0,
        state: "drafted",
        blocks: [{ kind: "paragraph", text: "Referred for attention concerns." }],
        citations: [],
        attempts: 1,
        error: null,
        updated_at: "2026-01-01T00:05:00Z",
      },
    ],
    instructions: [],
    writer_model_id: "model-9",
    finalized_revision: null,
    partial: false,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:05:00Z",
  };
}

const STOP_MESSAGE =
  "generate_full_report failed: The draft run was stopped.";

const SECTION_EVENTS: ReportTurnProgressView[] = [
  { kind: "plan_ready", section_count: 3 },
  { kind: "title_set", title: "Psychoeducational Evaluation" },
  { kind: "section_started", section_id: SECTION_ID, index: 0, total: 3 },
  {
    kind: "section_completed",
    section_id: SECTION_ID,
    section: {
      id: SECTION_ID,
      heading: "Reason for Referral",
      blocks: [{ kind: "paragraph", text: "Referred for attention concerns." }],
      skipped: false,
      template_blocks: null,
      authorship: null,
    },
    drafted: 1,
    total: 3,
  },
];

/** Start a generation and hand back the channel it is emitting through. */
async function startGeneration(hook: {
  result: { current: ReturnType<typeof useReportWorkspace> };
}) {
  let emit: ((progress: ReportTurnProgressView) => void) | undefined;
  let settle: ((response: FullReportGenerationResponse) => void) | undefined;
  let fail: ((error: unknown) => void) | undefined;
  mocks.generate.mockImplementation(
    (
      _clientId: string,
      _reportId: string,
      _revision: number,
      _modelId: string,
      _guidance: string,
      _streamId: string,
      onProgress?: (progress: ReportTurnProgressView) => void
    ) => {
      emit = onProgress;
      return new Promise<FullReportGenerationResponse>((resolve, reject) => {
        settle = resolve;
        fail = reject;
      });
    }
  );

  let generating: Promise<boolean> | undefined;
  await act(async () => {
    generating = hook.result.current.generateFullDraft("model-1", "");
  });
  return {
    emit: emit!,
    settle: settle!,
    fail: fail!,
    generating: () => generating!,
  };
}

function renderWorkspace() {
  return renderHook(() =>
    useReportWorkspace({ clientId: CLIENT_ID, expectedReportId: REPORT_ID })
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.load.mockResolvedValue(workspace());
  mocks.loadDraftRun.mockResolvedValue(null);
  mocks.stopStream.mockResolvedValue(undefined);
  mocks.loadConfig.mockResolvedValue({
    draft_pipeline: { plan_gate: "gated" },
  });
});

describe("useReportWorkspace drafting runs", () => {
  it("folds section progress into run state and the activity line", async () => {
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.loading).toBe(false));

    const run = await startGeneration(hook);
    expect(hook.result.current.run.live).toBe(true);

    act(() => {
      for (const event of SECTION_EVENTS) run.emit(event);
    });

    expect(hook.result.current.run.total).toBe(3);
    expect(hook.result.current.run.drafted).toBe(1);
    expect(hook.result.current.run.title).toBe("Psychoeducational Evaluation");
    expect(hook.result.current.run.sections.get(SECTION_ID)?.status).toBe(
      "drafted"
    );
    // The qualitative line names the section, not the tool.
    expect(hook.result.current.agentActivity).toEqual({
      label: "Saved “Reason for Referral”",
      detail: "section 1 of 3",
    });

    await act(async () => {
      run.settle(generationResponse());
      await run.generating();
    });
    // The command's own workspace clears the overlay.
    expect(hook.result.current.run.live).toBe(false);
    expect(hook.result.current.run.sections.size).toBe(0);
  });

  it("names the section it is drafting and how far along it is", async () => {
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    const run = await startGeneration(hook);

    act(() => {
      run.emit({
        kind: "section_started",
        section_id: SECTION_ID,
        index: 4,
        total: 9,
      });
    });

    expect(hook.result.current.agentActivity).toEqual({
      label: "Drafting “Reason for Referral”",
      detail: "section 5 of 9",
    });

    await act(async () => {
      run.settle(generationResponse());
      await run.generating();
    });
  });

  it("still routes the legacy tool events to the activity line", async () => {
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    const run = await startGeneration(hook);

    act(() => {
      run.emit({
        kind: "tool_started",
        name: "read_record_file",
        context: "intake.txt",
      });
    });
    expect(hook.result.current.agentActivity).toEqual({
      label: "Reading client context",
      detail: "intake.txt",
    });
    expect(hook.result.current.liveContext).toHaveLength(1);

    await act(async () => {
      run.settle(generationResponse());
      await run.generating();
    });
  });

  it("stops through the stream the generation minted", async () => {
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    const run = await startGeneration(hook);
    expect(hook.result.current.canStopRun).toBe(true);

    act(() => hook.result.current.stopRun());
    expect(hook.result.current.run.stopping).toBe(true);
    expect(hook.result.current.canStopRun).toBe(false);
    expect(mocks.stopStream).toHaveBeenCalledWith(
      mocks.generate.mock.calls[0][5]
    );

    await act(async () => {
      run.settle(generationResponse());
      await run.generating();
    });
  });

  it("shows a stopped run as an outcome rather than an error", async () => {
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    const run = await startGeneration(hook);
    mocks.loadDraftRun.mockResolvedValue(stoppedRun());

    await act(async () => {
      run.fail(STOP_MESSAGE);
      await run.generating();
    });

    expect(hook.result.current.actionError).toBeNull();
    expect(hook.result.current.run.outcome).toBe("stopped");
    expect(hook.result.current.run.drafted).toBe(1);
    expect(hook.result.current.run.total).toBe(1);
    expect(hook.result.current.busy).toBeNull();
    // Landed sections stay on the page while the reader decides.
    expect(hook.result.current.run.sections.get(SECTION_ID)?.status).toBe(
      "drafted"
    );
  });

  it("keeps the error path for a failure with no resumable run", async () => {
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    const run = await startGeneration(hook);
    mocks.loadDraftRun.mockResolvedValue(null);

    await act(async () => {
      run.fail("Bedrock is unavailable in this region.");
      await run.generating();
    });

    expect(hook.result.current.actionError).toContain(
      "Bedrock is unavailable in this region."
    );
    expect(hook.result.current.run.outcome).toBeNull();
    expect(mocks.logFrontendEvent).toHaveBeenCalledWith(
      "error",
      expect.stringContaining("Writer whole-report generation failed")
    );
  });

  it("keeps the error line above a failed run's outcome", async () => {
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    const run = await startGeneration(hook);
    mocks.loadDraftRun.mockResolvedValue({ ...stoppedRun(), status: "failed" });

    await act(async () => {
      run.fail("The model stopped responding.");
      await run.generating();
    });

    expect(hook.result.current.run.outcome).toBe("failed");
    expect(hook.result.current.actionError).toContain(
      "The model stopped responding."
    );
  });

  it("adopts a run stopped before the app was last closed", async () => {
    mocks.loadDraftRun.mockResolvedValue(stoppedRun());
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.loading).toBe(false));

    expect(mocks.loadDraftRun).toHaveBeenCalledWith(CLIENT_ID, REPORT_ID);
    expect(hook.result.current.run.outcome).toBe("stopped");
    expect(hook.result.current.run.runId).toBe(RUN_ID);
  });

  it("logs rather than hides a drafting-run lookup that fails", async () => {
    mocks.loadDraftRun.mockRejectedValue("network down");
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.loading).toBe(false));

    expect(hook.result.current.run.outcome).toBeNull();
    expect(hook.result.current.loadError).toBeNull();
    expect(mocks.logFrontendEvent).toHaveBeenCalledWith(
      "error",
      expect.stringContaining("Writer drafting-run lookup failed")
    );
  });

  it("resumes the adopted run with its own writer model", async () => {
    mocks.loadDraftRun.mockResolvedValue(stoppedRun());
    mocks.resumeDraftRun.mockResolvedValue(generationResponse());
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.run.outcome).toBe("stopped"));

    await act(async () => {
      await hook.result.current.resumeRun([], "  Tighten the summary.  ");
    });

    expect(mocks.resumeDraftRun).toHaveBeenCalledWith(
      CLIENT_ID,
      REPORT_ID,
      RUN_ID,
      "Tighten the summary.",
      "model-9",
      expect.any(String),
      expect.any(Function)
    );
    expect(hook.result.current.run.outcome).toBeNull();
    expect(hook.result.current.run.live).toBe(false);
  });

  it("resumes with no instructions when the box is left empty", async () => {
    mocks.loadDraftRun.mockResolvedValue(stoppedRun());
    mocks.resumeDraftRun.mockResolvedValue(generationResponse());
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.run.outcome).toBe("stopped"));

    await act(async () => {
      await hook.result.current.resumeRun([], "   ");
    });

    expect(mocks.resumeDraftRun.mock.calls[0][3]).toBeNull();
  });

  it("keeps a partial draft and clears the run", async () => {
    mocks.loadDraftRun.mockResolvedValue(stoppedRun());
    const finalized = workspace();
    finalized.draft.revision = 2;
    mocks.finalizePartialDraft.mockResolvedValue(finalized);
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.run.outcome).toBe("stopped"));

    await act(async () => {
      await hook.result.current.keepPartialDraft();
    });

    expect(mocks.finalizePartialDraft).toHaveBeenCalledWith(
      CLIENT_ID,
      REPORT_ID,
      RUN_ID
    );
    expect(hook.result.current.workspace?.draft.revision).toBe(2);
    expect(hook.result.current.run.outcome).toBeNull();
    expect(hook.result.current.saveStatus).toContain("revision 2");
  });

  it("discards a run and leaves the report where it was", async () => {
    mocks.loadDraftRun.mockResolvedValue(stoppedRun());
    mocks.abandonDraftRun.mockResolvedValue(workspace());
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.run.outcome).toBe("stopped"));

    await act(async () => {
      await hook.result.current.discardRun();
    });

    expect(mocks.abandonDraftRun).toHaveBeenCalledWith(
      CLIENT_ID,
      REPORT_ID,
      RUN_ID
    );
    expect(hook.result.current.run.outcome).toBeNull();
    expect(hook.result.current.workspace?.draft.revision).toBe(1);
  });

  it("keeps every run action referentially stable across renders", async () => {
    const hook = renderWorkspace();
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    const first = {
      stopRun: hook.result.current.stopRun,
      resumeRun: hook.result.current.resumeRun,
      keepPartialDraft: hook.result.current.keepPartialDraft,
      discardRun: hook.result.current.discardRun,
    };

    hook.rerender();

    expect(hook.result.current.stopRun).toBe(first.stopRun);
    expect(hook.result.current.resumeRun).toBe(first.resumeRun);
    expect(hook.result.current.keepPartialDraft).toBe(first.keepPartialDraft);
    expect(hook.result.current.discardRun).toBe(first.discardRun);
  });
});
