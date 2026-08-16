import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  FullReportGenerationResponse,
  ReportTurnProgressView,
  ReportWorkspaceView,
} from "./tauri";

const mocks = vi.hoisted(() => ({
  load: vi.fn(),
  generate: vi.fn(),
}));

vi.mock("./tauri", () => ({
  startReportWorkspace: mocks.load,
  loadReportWorkspace: mocks.load,
  generateFullReport: mocks.generate,
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

/**
 * The section-level progress kinds the drafting run emits. No UI reads them
 * yet — this file's job is to prove the writer hook stays exactly where it
 * was when they arrive.
 */
const SECTION_EVENTS: ReportTurnProgressView[] = [
  { kind: "plan_ready", section_count: 3 },
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
  { kind: "section_skipped", section_id: SECTION_ID, drafted: 2, total: 3 },
  {
    kind: "section_failed",
    section_id: SECTION_ID,
    message: "section_attempts_exhausted",
    drafted: 3,
    total: 3,
  },
  { kind: "title_set", title: "Psychoeducational Evaluation" },
];

describe("useReportWorkspace progress handling", () => {
  it("ignores section-level progress instead of folding it into tool activity", async () => {
    mocks.load.mockResolvedValue(workspace());
    let emit: ((progress: ReportTurnProgressView) => void) | undefined;
    mocks.generate.mockImplementation(
      async (
        _clientId: string,
        _reportId: string,
        _revision: number,
        _modelId: string,
        _guidance: string,
        _streamId: string,
        onProgress?: (progress: ReportTurnProgressView) => void
      ): Promise<FullReportGenerationResponse> => {
        emit = onProgress;
        // Hold the command open so the events land while the run is live,
        // which is the only window the frontend ever sees them in.
        await new Promise((resolve) => setTimeout(resolve, 0));
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
    );

    const hook = renderHook(() =>
      useReportWorkspace({ clientId: CLIENT_ID, expectedReportId: REPORT_ID })
    );
    await waitFor(() => expect(hook.result.current.loading).toBe(false));

    let generating: Promise<boolean> | undefined;
    await act(async () => {
      generating = hook.result.current.generateFullDraft("model-1", "");
    });
    expect(emit).toBeDefined();

    // A tool event the hook does understand, so there is a value to be
    // clobbered if the section kinds fell through to it.
    act(() => {
      emit?.({ kind: "tool_started", name: "write_full_draft_section", context: null });
    });
    expect(hook.result.current.agentActivity).toEqual({
      label: "Writing the complete report",
      detail: "Adding a section",
    });

    act(() => {
      for (const event of SECTION_EVENTS) emit?.(event);
    });

    expect(hook.result.current.agentActivity).toEqual({
      label: "Writing the complete report",
      detail: "Adding a section",
    });
    expect(hook.result.current.liveContext).toEqual([]);
    expect(hook.result.current.workspace?.draft.content.sections[0].blocks).toEqual([
      { kind: "paragraph", text: "Template boilerplate." },
    ]);
    expect(hook.result.current.actionError).toBeNull();

    await act(async () => {
      await generating;
    });
  });
});
