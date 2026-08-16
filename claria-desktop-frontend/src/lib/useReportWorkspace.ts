import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";
import {
  abandonDraftRun,
  applyReportTemplate,
  discardQueuedReportEdits,
  discardReportTemplatePreview,
  exportReportDocx,
  finalizePartialDraft,
  generateDraftPlan,
  generateFullReport,
  loadConfig,
  loadDraftRun,
  loadReportWorkspace,
  previewWriterTemplate,
  renameReportSession,
  resolveReportProposal,
  resumeDraftRun,
  saveReportDraft,
  sendReportMessage,
  startDraftRun,
  startReportWorkspace,
  stopStream,
  updateDraftPlan,
  type DraftRun,
  type FullReportGenerationResponse,
  type PlanEntryEdit,
  type ReportDraftEdit,
  type ReportTurnProgressView,
  type ReportWorkspaceView,
} from "./tauri";
import type { DraftPlanMode } from "./draftPlan";
import { randomUuid } from "./ids";
import { useAsyncLoad } from "./useAsyncLoad";
import {
  countReportEdits,
  draftToEdit,
  reportEditsEqual,
  validateReportEdit,
} from "./writingWorkspace";
import {
  emptyDraftRun,
  reduceDraftRun,
  runStateFromDraftRun,
  type DraftRunUiState,
} from "./draftRun";
import { upsertLiveContext, type ContextPill } from "./contextPills";
import { logFrontendEvent } from "./logBridge";

export type WriterBusy =
  | null
  | "saving"
  | "discarding"
  | "sending"
  | "planning"
  | "generating"
  | "resolving"
  | "exporting"
  | "applying_template";

/** What the Draft run pane shows, or `null` when this report has no run. */
export type DraftPaneState = {
  mode: DraftPlanMode;
  /** The run behind the pane; `null` only while the plan pass is still out. */
  run: DraftRun | null;
} | null;

export type AgentActivity = { label: string; detail?: string } | null;

function isReportConflict(message: string): boolean {
  return message.toLowerCase().includes("changed on another computer");
}

/**
 * The progress channel drives the run state; everything else about a run —
 * starting it, picking it back up, and adopting durable state after it ends —
 * is one of these.
 */
type RunAction =
  | { kind: "progress"; event: ReportTurnProgressView }
  | { kind: "started" }
  | { kind: "resumed" }
  | { kind: "stopping" }
  | { kind: "hydrated"; state: DraftRunUiState }
  | { kind: "cleared" };

function runReducer(
  state: DraftRunUiState,
  action: RunAction
): DraftRunUiState {
  switch (action.kind) {
    case "progress":
      return reduceDraftRun(state, action.event);
    case "started":
      return { ...emptyDraftRun(), live: true };
    case "resumed":
      // Sections the earlier attempt landed stay on screen; the run reports
      // its own counters from its first event.
      return { ...state, live: true, stopping: false, outcome: null };
    case "stopping":
      return state.stopping ? state : { ...state, stopping: true };
    case "hydrated":
      return action.state;
    case "cleared":
      return emptyDraftRun();
  }
}

const MODEL_ACTIVITY_WORDS = ["thinking", "working", "inferring"] as const;

function modelActivityLabel(callNumber: number): string {
  const index = Math.max(0, callNumber - 1) % MODEL_ACTIVITY_WORDS.length;
  return `Claude is ${MODEL_ACTIVITY_WORDS[index]}`;
}

function agentActivityForTool(name: string, context: string | null) {
  if (name === "list_record_files") {
    return { label: "Checking available records", detail: context ?? undefined };
  }
  if (name === "read_record_file") {
    return { label: "Reading client context", detail: context ?? undefined };
  }
  // The targeted-edit context restructure ships on a sibling branch; until it
  // merges no turn emits this name, and the label is inert.
  if (name === "read_report_section") {
    return { label: "Reading a report section", detail: context ?? undefined };
  }
  if (name === "propose_report_changes") {
    return { label: "Drafting a reviewable proposal" };
  }
  if (name === "set_full_draft_title") {
    return { label: "Setting the complete report title" };
  }
  if (name === "write_full_draft_section") {
    return { label: "Writing the complete report", detail: "Adding a section" };
  }
  if (name === "finish_full_draft") {
    return { label: "Validating the complete report" };
  }
  return { label: "Using an approved tool", detail: name };
}

function sectionHeading(
  workspace: ReportWorkspaceView | null,
  sectionId: string
): string | null {
  return (
    workspace?.draft.content.sections.find(
      (section) => section.id === sectionId
    )?.heading ?? null
  );
}

/**
 * The qualitative line for one section-level progress event. `null` for the
 * events that only move counters, so the previous line keeps standing.
 */
function agentActivityForSection(
  progress: ReportTurnProgressView,
  headingFor: (sectionId: string) => string | null
): AgentActivity {
  const named = (sectionId: string, verb: string, index: number, total: number) => {
    const heading = headingFor(sectionId);
    return {
      label: heading ? `${verb} “${heading}”` : verb,
      detail: `section ${index} of ${total}`,
    };
  };
  switch (progress.kind) {
    case "plan_ready":
      return {
        label: "Planning the report",
        detail: `${progress.section_count} section${progress.section_count === 1 ? "" : "s"} to decide`,
      };
    case "section_started":
      return named(
        progress.section_id,
        "Drafting",
        progress.index + 1,
        progress.total
      );
    case "section_completed":
      return named(
        progress.section_id,
        "Saved",
        progress.drafted,
        progress.total
      );
    case "section_skipped":
      return named(
        progress.section_id,
        "Skipped",
        progress.drafted,
        progress.total
      );
    case "section_failed":
      return {
        label: headingFor(progress.section_id)
          ? `Could not write “${headingFor(progress.section_id)}”`
          : "Could not write a section",
        detail: progress.message,
      };
    default:
      return null;
  }
}

/**
 * Everything the writer surface knows about its report workspace: loading,
 * the inline edit buffer, and the load/save/send/resolve/export/apply-template
 * actions with the stale-generation guard inside.
 *
 * Every returned action is referentially stable — the current state is read
 * through a ref — so callers can hand them to memoized children directly.
 * Actions that a caller reacts to (send, applyTemplate) return `true` on
 * success and `false` when they failed or were superseded.
 */
export function useReportWorkspace({
  clientId,
  expectedReportId,
}: {
  clientId: string;
  expectedReportId?: string | null;
}) {
  const generationRef = useRef(0);
  const activeReportIdRef = useRef(expectedReportId ?? randomUuid());
  const startingNewSessionRef = useRef(expectedReportId == null);
  const [workspace, setWorkspace] = useState<ReportWorkspaceView | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [busy, setBusy] = useState<WriterBusy>(null);
  const [editing, setEditing] = useState(false);
  const [edit, setEdit] = useState<ReportDraftEdit>({
    title: "Untitled report",
    sections: [],
  });
  const [saveStatus, setSaveStatus] = useState<string | null>(null);
  const [exportStatus, setExportStatus] = useState<string | null>(null);
  const [agentActivity, setAgentActivity] = useState<AgentActivity>(null);
  const [liveContext, setLiveContext] = useState<ContextPill[]>([]);
  const [run, dispatchRun] = useReducer(runReducer, undefined, emptyDraftRun);
  const [draftPane, setDraftPane] = useState<DraftPaneState>(null);
  // The durable run behind `run.runId`: resume needs its writer model, and
  // finalize and abandon need its ID after the reducer has moved on.
  const resumableRunRef = useRef<DraftRun | null>(null);
  // Minted before every run command is invoked, so Stop is live from the
  // moment the run is — planning, drafting, and picking back up alike.
  const streamIdRef = useRef<string | null>(null);

  // Whether a planned draft stops at the gate is a saved preference, read
  // once per writer surface. Gated is the safe reading of an unknown answer:
  // it never starts an expensive draft nobody approved.
  const configLoad = useAsyncLoad(() => loadConfig(), []);
  const configError = configLoad.error;
  useEffect(() => {
    if (configError) {
      logFrontendEvent(
        "warn",
        `Writer plan-gate preference unavailable, gating the draft: ${configError}`
      );
    }
  }, [configError]);
  const planGate = configLoad.data?.draft_pipeline.plan_gate ?? "gated";

  const baseline = useMemo(
    () => (workspace ? draftToEdit(workspace.draft) : null),
    [workspace]
  );
  const dirty = Boolean(
    editing && baseline && !reportEditsEqual(edit, baseline)
  );
  const editCount = useMemo(
    () => (baseline && dirty ? countReportEdits(baseline, edit) : 0),
    [baseline, dirty, edit]
  );
  const validationErrors = useMemo(() => validateReportEdit(edit), [edit]);
  const savedEditsQueued = Boolean(
    workspace &&
      workspace.draft.revision > (workspace.last_agent_revision ?? 0)
  );

  // Actions outlive the renders that created them; they read the latest
  // state through this ref so they can stay referentially stable.
  const stateRef = useRef({
    workspace,
    edit,
    dirty,
    busy,
    validationErrors,
    planGate,
  });
  stateRef.current = {
    workspace,
    edit,
    dirty,
    busy,
    validationErrors,
    planGate,
  };

  const showActionError = useCallback((error: unknown) => {
    const message = String(error);
    setActionError(message);
    setConflict(isReportConflict(message));
  }, []);

  /**
   * Adopt whatever durable run this report has, if it is one the writer can
   * pick back up. Returns the run's outcome so a caller can decide whether a
   * failed command still deserves an error line.
   */
  const adoptDraftRun = useCallback(
    async (reportId: string, generation: number) => {
      let durable: DraftRun | null = null;
      try {
        durable = await loadDraftRun(clientId, reportId);
      } catch (error) {
        // A run we cannot read is a run we cannot offer; say so in the log
        // rather than pretending the report has none.
        logFrontendEvent(
          "error",
          `Writer drafting-run lookup failed: ${String(error)}`
        );
        return null;
      }
      if (generation !== generationRef.current) return null;
      const hydrated = durable ? runStateFromDraftRun(durable) : null;
      if (durable && durable.status === "awaiting_approval" && durable.plan) {
        // A plan the reader never answered still holds the report. Offering
        // it back is the only way out of that hold that is not a refusal.
        resumableRunRef.current = durable;
        setDraftPane({ mode: "plan-gate", run: durable });
        return null;
      }
      if (!durable || !hydrated?.outcome) {
        resumableRunRef.current = null;
        setDraftPane(null);
        return null;
      }
      resumableRunRef.current = durable;
      dispatchRun({ kind: "hydrated", state: hydrated });
      setDraftPane({ mode: "resume-gate", run: durable });
      return hydrated.outcome;
    },
    [clientId]
  );

  const load = useCallback(async () => {
    const generation = ++generationRef.current;
    setLoading(true);
    setLoadError(null);
    setActionError(null);
    setConflict(false);
    try {
      const result = startingNewSessionRef.current
        ? await startReportWorkspace(clientId, activeReportIdRef.current)
        : await loadReportWorkspace(clientId, activeReportIdRef.current);
      if (generation !== generationRef.current) return;
      if (expectedReportId && result.report_id !== expectedReportId) {
        throw new Error("That Editor History session is no longer available.");
      }
      activeReportIdRef.current = result.report_id;
      startingNewSessionRef.current = false;
      setWorkspace(result);
      setEdit(draftToEdit(result.draft));
      setEditing(false);
      resumableRunRef.current = null;
      setDraftPane(null);
      dispatchRun({ kind: "cleared" });
      // A run stopped before the app was closed is still resumable, so the
      // banner has to survive a restart, not just a re-render.
      await adoptDraftRun(result.report_id, generation);
    } catch (error) {
      if (generation !== generationRef.current) return;
      setLoadError(String(error));
    } finally {
      if (generation === generationRef.current) setLoading(false);
    }
  }, [adoptDraftRun, clientId, expectedReportId]);

  useEffect(() => {
    void load();
    return () => {
      generationRef.current += 1;
    };
  }, [load]);

  const beginEdit = useCallback(() => {
    const current = stateRef.current.workspace;
    if (!current) return;
    setEdit(draftToEdit(current.draft));
    setEditing(true);
    setActionError(null);
    setSaveStatus(null);
  }, []);

  const cancelEdit = useCallback(() => {
    const current = stateRef.current.workspace;
    if (!current) return;
    setEdit(draftToEdit(current.draft));
    setEditing(false);
    setActionError(null);
  }, []);

  /** Save any dirty inline edit, returning the workspace to act against. */
  const persistCurrentEdit = useCallback(async (): Promise<ReportWorkspaceView> => {
    const { workspace: current, dirty: isDirty, edit: currentEdit, validationErrors: errors } =
      stateRef.current;
    if (!current) throw new Error("The writing session is not loaded.");
    if (!isDirty) return current;
    if (errors.length > 0) {
      throw new Error("Fix the highlighted report fields before continuing.");
    }
    // TODO(#73 FE): switch to a named-field patch-save command once the
    // backend exposes one, so a section saves only its own fields instead of
    // this positional whole-draft write.
    const result = await saveReportDraft(
      clientId,
      current.report_id,
      current.draft.revision,
      currentEdit
    );
    setWorkspace(result);
    setEdit(draftToEdit(result.draft));
    setConflict(false);
    return result;
  }, [clientId]);

  const save = useCallback(async () => {
    const { dirty: isDirty, busy: currentBusy, validationErrors: errors } =
      stateRef.current;
    if (!isDirty || currentBusy !== null || errors.length > 0) return;
    const generation = generationRef.current;
    setBusy("saving");
    setActionError(null);
    setSaveStatus("Saving report edits…");
    try {
      const result = await persistCurrentEdit();
      if (generation !== generationRef.current) return;
      setEditing(false);
      setSaveStatus(
        `Saved revision ${result.draft.revision}. These edits are queued for Claude's next message.`
      );
    } catch (error) {
      if (generation !== generationRef.current) return;
      showActionError(error);
      setSaveStatus(null);
    } finally {
      if (generation === generationRef.current) setBusy(null);
    }
  }, [persistCurrentEdit, showActionError]);

  const discardQueuedEdits = useCallback(async () => {
    const { workspace: current, dirty: isDirty, busy: currentBusy } =
      stateRef.current;
    if (!current || currentBusy !== null) return;
    const hasSavedEdits =
      current.draft.revision > (current.last_agent_revision ?? 0);
    if (!isDirty && !hasSavedEdits) return;

    const generation = generationRef.current;
    setBusy("discarding");
    setActionError(null);
    setSaveStatus("Discarding queued report edits…");
    try {
      const result = hasSavedEdits
        ? await discardQueuedReportEdits(
            clientId,
            current.report_id,
            current.draft.revision
          )
        : current;
      if (generation !== generationRef.current) return;
      setWorkspace(result);
      setEdit(draftToEdit(result.draft));
      setEditing(false);
      setConflict(false);
      setSaveStatus(
        hasSavedEdits
          ? `Discarded queued edits and restored the report as revision ${result.draft.revision}.`
          : "Discarded unsaved report edits."
      );
    } catch (error) {
      if (generation !== generationRef.current) return;
      showActionError(error);
      setSaveStatus(null);
    } finally {
      if (generation === generationRef.current) setBusy(null);
    }
  }, [clientId, showActionError]);

  const handleAgentProgress = useCallback((progress: ReportTurnProgressView) => {
    if (progress.kind === "record_context_prepared") {
      setAgentActivity({
        label: `Loaded ${progress.included_files} readable record${progress.included_files === 1 ? "" : "s"}`,
        detail:
          progress.unavailable_files > 0
            ? `${progress.unavailable_files} unavailable · ${progress.total_characters.toLocaleString()} characters ready`
            : `${progress.total_characters.toLocaleString()} characters ready`,
      });
      return;
    }

    if (progress.kind === "model_call_started") {
      setAgentActivity({
        label: modelActivityLabel(progress.call_number),
        detail: `Model call ${progress.call_number}`,
      });
      return;
    }

    // Section-level progress drives the canvas through the run reducer. It
    // also carries the qualitative line, because "Drafting section 5 of 9" is
    // a better answer to "what is it doing" than any tool name.
    if (progress.kind !== "tool_started" && progress.kind !== "tool_finished") {
      dispatchRun({ kind: "progress", event: progress });
      const activity = agentActivityForSection(progress, (sectionId) =>
        sectionHeading(stateRef.current.workspace, sectionId)
      );
      if (activity) setAgentActivity(activity);
      return;
    }

    const context = progress.context;
    if (progress.kind === "tool_started") {
      setAgentActivity(agentActivityForTool(progress.name, context));
      if (context && !progress.name.includes("full_draft")) {
        setLiveContext((current) => upsertLiveContext(current, context, "loading"));
      }
      return;
    }

    setAgentActivity(
      progress.name === "propose_report_changes"
        ? { label: "Claude is reviewing its proposal" }
        : progress.name === "finish_full_draft"
          ? { label: "Complete working draft ready" }
          : progress.name === "write_full_draft_section"
            ? { label: "Complete report section ready" }
            : {
                label:
                  progress.status === "succeeded" ? "Context ready" : "Context unavailable",
                detail: context ?? progress.name,
              }
    );
    if (context && !progress.name.includes("full_draft")) {
      setLiveContext((current) =>
        upsertLiveContext(
          current,
          context,
          progress.status === "succeeded" ? "ready" : "failed"
        )
      );
    }
  }, []);

  const send = useCallback(
    async (
      modelId: string,
      instruction: string,
      references: Array<{ section_id: string; block_index: number }>
    ): Promise<boolean> => {
      const generation = generationRef.current;
      setBusy("sending");
      setActionError(null);
      setLiveContext([]);
      setAgentActivity({
        label: "Preparing the writer",
        detail: "Loading approved context",
      });
      setSaveStatus(
        stateRef.current.dirty
          ? "Saving your report edits before Claude reads the next message…"
          : "Claude is using the approved tools…"
      );
      try {
        const current = await persistCurrentEdit();
        if (generation !== generationRef.current) return false;
        const result = await sendReportMessage(
          clientId,
          current.report_id,
          current.draft.revision,
          modelId,
          instruction,
          references,
          randomUuid(),
          (progress) => {
            if (generation === generationRef.current) handleAgentProgress(progress);
          }
        );
        if (generation !== generationRef.current) return false;
        setWorkspace(result.workspace);
        setEdit(draftToEdit(result.workspace.draft));
        setConflict(false);
        setSaveStatus(
          result.workspace.pending_proposal
            ? "Proposal ready for your review. The accepted draft is unchanged."
            : "Writing assistant turn complete."
        );
        setLiveContext([]);
        return true;
      } catch (error) {
        if (generation !== generationRef.current) return false;
        // Keep the caller's instruction and references for an exact retry.
        showActionError(error);
        setSaveStatus(null);
        setLiveContext([]);
        return false;
      } finally {
        if (generation === generationRef.current) {
          setAgentActivity(null);
          setBusy(null);
        }
      }
    },
    [clientId, handleAgentProgress, persistCurrentEdit, showActionError]
  );

  /**
   * The tail shared by starting a run and picking one back up: adopt the
   * finished workspace, or work out from durable state whether the failure was
   * a stop the reader asked for.
   *
   * The stop message is never matched. The run object decides: if the report
   * still holds a run the writer can pick back up, that is the outcome and the
   * banner says so; anything else is a real error.
   */
  const settleRun = useCallback(
    async (
      generation: number,
      command: Promise<FullReportGenerationResponse>
    ): Promise<boolean> => {
      const reportId = stateRef.current.workspace?.report_id;
      try {
        const result = await command;
        if (generation !== generationRef.current) return false;
        setWorkspace(result.workspace);
        setEdit(draftToEdit(result.workspace.draft));
        setEditing(false);
        setConflict(false);
        resumableRunRef.current = null;
        setDraftPane(null);
        dispatchRun({ kind: "cleared" });
        const unavailable =
          result.unavailable_record_files > 0
            ? ` ${result.unavailable_record_files} record${result.unavailable_record_files === 1 ? " was" : "s were"} unavailable and listed in the writer context.`
            : "";
        setSaveStatus(
          `Generated and saved revision ${result.workspace.draft.revision} from ${result.included_record_files} readable record${result.included_record_files === 1 ? "" : "s"}.${unavailable}`
        );
        setLiveContext([]);
        return true;
      } catch (error) {
        const diagnostic =
          error instanceof Error ? (error.stack ?? error.message) : String(error);
        logFrontendEvent(
          "error",
          `Writer whole-report generation failed: ${diagnostic}`
        );
        const outcome = reportId
          ? await adoptDraftRun(reportId, generation)
          : null;
        if (generation !== generationRef.current) return false;
        setSaveStatus(null);
        setLiveContext([]);
        if (outcome === "stopped") {
          // The banner carries the outcome; an error toast on top of it would
          // frame the reader's own Stop as a fault.
          setActionError(null);
          setConflict(false);
          return false;
        }
        if (!outcome) {
          dispatchRun({ kind: "cleared" });
          setDraftPane(null);
        }
        showActionError(error);
        return false;
      } finally {
        if (generation === generationRef.current) {
          setAgentActivity(null);
          setBusy(null);
          streamIdRef.current = null;
        }
      }
    },
    [adoptDraftRun, showActionError]
  );

  const generateFullDraft = useCallback(
    async (modelId: string, guidance: string): Promise<boolean> => {
      const { busy: currentBusy } = stateRef.current;
      if (currentBusy !== null) return false;
      const generation = generationRef.current;
      // Minted before the first render of the busy state, so the Stop button
      // is live from the moment the run is.
      const streamId = randomUuid();
      streamIdRef.current = streamId;
      setBusy("generating");
      setActionError(null);
      setLiveContext([]);
      dispatchRun({ kind: "started" });
      setAgentActivity({
        label: "Preparing the complete report",
        detail: "Loading every readable client record",
      });
      setSaveStatus(
        stateRef.current.dirty
          ? "Saving your report edits before generating the complete draft…"
          : "Loading all readable records for the complete draft…"
      );
      logFrontendEvent("info", "Writer whole-report generation requested");
      let current: ReportWorkspaceView;
      try {
        current = await persistCurrentEdit();
      } catch (error) {
        if (generation !== generationRef.current) return false;
        showActionError(error);
        setSaveStatus(null);
        setLiveContext([]);
        dispatchRun({ kind: "cleared" });
        setAgentActivity(null);
        setBusy(null);
        streamIdRef.current = null;
        return false;
      }
      if (generation !== generationRef.current) return false;
      return settleRun(
        generation,
        generateFullReport(
          clientId,
          current.report_id,
          current.draft.revision,
          modelId,
          guidance,
          streamId,
          (progress) => {
            if (generation === generationRef.current) handleAgentProgress(progress);
          }
        )
      );
    },
    [
      clientId,
      handleAgentProgress,
      persistCurrentEdit,
      settleRun,
      showActionError,
    ]
  );

  /**
   * Flush the reader's gate edits, then draft the plan they approved.
   *
   * The edits go in one batched call carrying only the rows that changed, and
   * the run it returns replaces the pane's copy — so the plan shown beside the
   * progress is the plan the writer was actually handed.
   */
  const startPlannedRun = useCallback(
    async (
      modelId: string,
      planned: DraftRun,
      edits: PlanEntryEdit[]
    ): Promise<boolean> => {
      const current = stateRef.current.workspace;
      if (!current) return false;
      const generation = generationRef.current;
      const streamId = randomUuid();
      streamIdRef.current = streamId;
      setBusy("generating");
      setActionError(null);
      setLiveContext([]);
      setAgentActivity({
        label: "Preparing the complete report",
        detail: "Loading every readable client record",
      });
      setSaveStatus("Drafting the report from the plan you approved…");
      logFrontendEvent("info", "Writer planned drafting run requested");

      let approved = planned;
      if (edits.length > 0) {
        try {
          approved = await updateDraftPlan(
            clientId,
            current.report_id,
            planned.run_id,
            edits
          );
        } catch (error) {
          if (generation !== generationRef.current) return false;
          showActionError(error);
          setSaveStatus(null);
          setAgentActivity(null);
          setBusy(null);
          streamIdRef.current = null;
          return false;
        }
        if (generation !== generationRef.current) return false;
      }
      resumableRunRef.current = approved;
      setDraftPane({ mode: "running", run: approved });
      dispatchRun({ kind: "started" });
      return settleRun(
        generation,
        startDraftRun(
          clientId,
          current.report_id,
          approved.run_id,
          modelId,
          streamId,
          (progress) => {
            if (generation === generationRef.current) handleAgentProgress(progress);
          }
        )
      );
    },
    [clientId, handleAgentProgress, settleRun, showActionError]
  );

  /**
   * Plan a whole-report draft from every readable record.
   *
   * Where the plan lands is the reader's saved preference: gated leaves it in
   * the Draft run pane to be read and edited, auto-start hands it straight to
   * the writer. The chaining lives here rather than in a backend command so
   * each command stays one job.
   */
  const planRun = useCallback(
    async (modelId: string, guidance: string): Promise<boolean> => {
      const { busy: currentBusy } = stateRef.current;
      if (currentBusy !== null) return false;
      const generation = generationRef.current;
      // Minted before the first render of the busy state, so Stop is live
      // from the moment the plan pass is.
      const streamId = randomUuid();
      streamIdRef.current = streamId;
      setBusy("planning");
      setActionError(null);
      setLiveContext([]);
      dispatchRun({ kind: "started" });
      setDraftPane({ mode: "running", run: null });
      setAgentActivity({
        label: "Planning the report",
        detail: "Deciding what each section needs",
      });
      setSaveStatus(
        stateRef.current.dirty
          ? "Saving your report edits before planning the complete draft…"
          : "Reading every readable record to plan the complete draft…"
      );
      logFrontendEvent("info", "Writer draft plan requested");

      const abandonAttempt = (error: unknown) => {
        showActionError(error);
        setSaveStatus(null);
        setLiveContext([]);
        setDraftPane(null);
        dispatchRun({ kind: "cleared" });
        setAgentActivity(null);
        setBusy(null);
        streamIdRef.current = null;
      };

      let current: ReportWorkspaceView;
      try {
        current = await persistCurrentEdit();
      } catch (error) {
        if (generation !== generationRef.current) return false;
        abandonAttempt(error);
        return false;
      }
      if (generation !== generationRef.current) return false;

      let planned: DraftRun;
      try {
        planned = await generateDraftPlan(
          clientId,
          current.report_id,
          current.draft.revision,
          guidance,
          streamId,
          (progress) => {
            if (generation === generationRef.current) handleAgentProgress(progress);
          }
        );
      } catch (error) {
        const diagnostic =
          error instanceof Error ? (error.stack ?? error.message) : String(error);
        logFrontendEvent("error", `Writer draft planning failed: ${diagnostic}`);
        if (generation !== generationRef.current) return false;
        abandonAttempt(error);
        return false;
      }
      if (generation !== generationRef.current) return false;

      if (stateRef.current.planGate === "gated") {
        resumableRunRef.current = planned;
        setDraftPane({ mode: "plan-gate", run: planned });
        // The plan pass moved the counters; the gate is not drafting, so the
        // canvas goes quiet again until the reader approves.
        dispatchRun({ kind: "cleared" });
        setAgentActivity(null);
        setBusy(null);
        streamIdRef.current = null;
        setSaveStatus(
          `Plan ready. Review ${planned.sections.length} section${planned.sections.length === 1 ? "" : "s"} before drafting starts.`
        );
        return true;
      }
      return startPlannedRun(modelId, planned, []);
    },
    [
      clientId,
      handleAgentProgress,
      persistCurrentEdit,
      showActionError,
      startPlannedRun,
    ]
  );

  /** Show the interrupted run's plan so its directives can be changed. */
  const openResumeGate = useCallback(() => {
    const durable = resumableRunRef.current;
    if (!durable) return;
    setDraftPane({ mode: "resume-gate", run: durable });
  }, []);

  /**
   * Ask the backend to end the run. Nothing here decides the outcome — the
   * in-flight command settling does, which is what makes a stop that loses
   * its race with a finishing run leave the finished revision standing.
   */
  const stopRun = useCallback(() => {
    const streamId = streamIdRef.current;
    if (!streamId) return;
    dispatchRun({ kind: "stopping" });
    logFrontendEvent("info", "Writer run stop requested");
    void stopStream(streamId).catch((error) => {
      logFrontendEvent("error", `Writer run stop failed: ${String(error)}`);
    });
  }, []);

  /**
   * Pick the interrupted run back up, carrying whatever the reader changed at
   * the resume gate. Changing nothing is one command and one round trip: the
   * backend's own deterministic defaults are already what the gate showed.
   */
  const resumeRun = useCallback(
    async (
      edits: PlanEntryEdit[] = [],
      instructions = ""
    ): Promise<boolean> => {
      const { workspace: current, busy: currentBusy } = stateRef.current;
      const durable = resumableRunRef.current;
      if (!current || currentBusy !== null || !durable) return false;
      const generation = generationRef.current;
      const streamId = randomUuid();
      streamIdRef.current = streamId;
      setBusy("generating");
      setActionError(null);
      setLiveContext([]);
      dispatchRun({ kind: "resumed" });
      setAgentActivity({
        label: "Picking the draft back up",
        detail: "Deciding what is left to write",
      });
      setSaveStatus("Picking the interrupted draft back up…");
      logFrontendEvent("info", "Writer drafting run resume requested");

      let approved = durable;
      if (edits.length > 0) {
        try {
          approved = await updateDraftPlan(
            clientId,
            current.report_id,
            durable.run_id,
            edits
          );
        } catch (error) {
          if (generation !== generationRef.current) return false;
          showActionError(error);
          setSaveStatus(null);
          setAgentActivity(null);
          setBusy(null);
          streamIdRef.current = null;
          return false;
        }
        if (generation !== generationRef.current) return false;
        resumableRunRef.current = approved;
      }
      setDraftPane({ mode: "running", run: approved });
      const trimmed = instructions.trim();
      return settleRun(
        generation,
        resumeDraftRun(
          clientId,
          current.report_id,
          approved.run_id,
          trimmed === "" ? null : trimmed,
          approved.writer_model_id,
          streamId,
          (progress) => {
            if (generation === generationRef.current) handleAgentProgress(progress);
          }
        )
      );
    },
    [clientId, handleAgentProgress, settleRun, showActionError]
  );

  /** Keep what an interrupted run wrote; undone sections become skipped. */
  const keepPartialDraft = useCallback(async (): Promise<boolean> => {
    const { workspace: current, busy: currentBusy } = stateRef.current;
    const durable = resumableRunRef.current;
    if (!current || currentBusy !== null || !durable) return false;
    const generation = generationRef.current;
    setBusy("resolving");
    setActionError(null);
    setSaveStatus("Keeping the sections the draft finished…");
    try {
      const result = await finalizePartialDraft(
        clientId,
        current.report_id,
        durable.run_id
      );
      if (generation !== generationRef.current) return false;
      setWorkspace(result);
      setEdit(draftToEdit(result.draft));
      setEditing(false);
      setConflict(false);
      resumableRunRef.current = null;
      setDraftPane(null);
      dispatchRun({ kind: "cleared" });
      setSaveStatus(
        `Kept the partial draft as revision ${result.draft.revision}. Sections the run never wrote are marked skipped.`
      );
      return true;
    } catch (error) {
      if (generation !== generationRef.current) return false;
      showActionError(error);
      setSaveStatus(null);
      return false;
    } finally {
      if (generation === generationRef.current) setBusy(null);
    }
  }, [clientId, showActionError]);

  /** Throw the interrupted run away. The report keeps its revision. */
  const discardRun = useCallback(async (): Promise<boolean> => {
    const { workspace: current, busy: currentBusy } = stateRef.current;
    const durable = resumableRunRef.current;
    if (!current || currentBusy !== null || !durable) return false;
    const generation = generationRef.current;
    setBusy("discarding");
    setActionError(null);
    setSaveStatus("Discarding the interrupted draft…");
    try {
      const result = await abandonDraftRun(
        clientId,
        current.report_id,
        durable.run_id
      );
      if (generation !== generationRef.current) return false;
      setWorkspace(result);
      setEdit(draftToEdit(result.draft));
      setEditing(false);
      setConflict(false);
      resumableRunRef.current = null;
      setDraftPane(null);
      dispatchRun({ kind: "cleared" });
      setSaveStatus(
        `Discarded the interrupted draft. The report is unchanged at revision ${result.draft.revision}.`
      );
      return true;
    } catch (error) {
      if (generation !== generationRef.current) return false;
      showActionError(error);
      setSaveStatus(null);
      return false;
    } finally {
      if (generation === generationRef.current) setBusy(null);
    }
  }, [clientId, showActionError]);

  const resolveProposal = useCallback(
    async (decision: "accept" | "reject") => {
      const { workspace: current, busy: currentBusy } = stateRef.current;
      if (!current?.pending_proposal || currentBusy !== null) return;
      const generation = generationRef.current;
      const proposalId = current.pending_proposal.id;
      setBusy("resolving");
      setActionError(null);
      setSaveStatus(
        decision === "accept"
          ? "Accepting and saving proposal…"
          : "Rejecting proposal…"
      );
      try {
        const result = await resolveReportProposal(
          clientId,
          current.report_id,
          proposalId,
          decision
        );
        if (generation !== generationRef.current) return;
        setWorkspace(result);
        setEdit(draftToEdit(result.draft));
        setEditing(false);
        setConflict(false);
        setSaveStatus(
          decision === "accept"
            ? `Accepted and saved as revision ${result.draft.revision}.`
            : "Proposal rejected. The accepted draft was not changed."
        );
      } catch (error) {
        if (generation !== generationRef.current) return;
        showActionError(error);
        setSaveStatus(null);
      } finally {
        if (generation === generationRef.current) setBusy(null);
      }
    },
    [clientId, showActionError]
  );

  const applyTemplate = useCallback(
    async (templateId: string): Promise<boolean> => {
      const { workspace: current, busy: currentBusy, dirty: isDirty } =
        stateRef.current;
      if (
        !current ||
        !templateId ||
        currentBusy !== null ||
        isDirty ||
        current.pending_proposal
      ) {
        return false;
      }
      const generation = generationRef.current;
      let importId: string | null = null;
      setBusy("applying_template");
      setActionError(null);
      setSaveStatus("Applying the managed Word template…");
      try {
        const preview = await previewWriterTemplate(clientId, templateId);
        importId = preview.import_id;
        if (generation !== generationRef.current) return false;
        const result = await applyReportTemplate(
          clientId,
          current.report_id,
          current.draft.revision,
          preview.import_id
        );
        if (generation !== generationRef.current) return false;
        setWorkspace(result);
        setEdit(draftToEdit(result.draft));
        setEditing(false);
        setConflict(false);
        setSaveStatus(
          `Applied the Word template as revision ${result.draft.revision}. Its layout and formatting will be retained on export.`
        );
        return true;
      } catch (error) {
        if (generation !== generationRef.current) return false;
        showActionError(error);
        setSaveStatus(null);
        return false;
      } finally {
        if (importId) {
          await discardReportTemplatePreview(importId).catch(() => undefined);
        }
        if (generation === generationRef.current) setBusy(null);
      }
    },
    [clientId, showActionError]
  );

  const exportDocx = useCallback(async () => {
    const { workspace: current, busy: currentBusy, dirty: isDirty } =
      stateRef.current;
    if (isDirty || currentBusy !== null || !current) return;
    const generation = generationRef.current;
    const visibleReportId = current.report_id;
    const visibleRevision = current.draft.revision;
    setBusy("exporting");
    setActionError(null);
    setSaveStatus(null);
    setExportStatus("Choose where to save the Word document…");
    try {
      const result = await exportReportDocx(
        clientId,
        visibleReportId,
        visibleRevision
      );
      if (generation !== generationRef.current) return;
      if (
        result.report_id !== visibleReportId ||
        result.revision !== visibleRevision
      ) {
        throw new Error("The exported report revision did not match the visible report.");
      }
      setWorkspace((value) =>
        value
          ? {
              ...value,
              last_export: {
                revision: result.revision,
                status: result.status,
                attempted_at: result.attempted_at,
              },
            }
          : value
      );
      setConflict(false);
      const persistenceSuffix = result.status_persisted
        ? ""
        : " Export status could not be synced to Editor History.";
      // Reduced template fidelity is reported, never silently degraded.
      const templateSuffix =
        result.template_warning === "template_missing"
          ? " The imported template's stored formatting was unavailable, so default formatting was used."
          : result.template_warning === "template_body_fallback"
            ? " The template's body layout could not be reused exactly, so default body formatting was used inside it."
            : "";
      setExportStatus(
        result.exported
          ? `Word document exported from revision ${result.revision}.${templateSuffix}${persistenceSuffix}`
          : `Export canceled. You can try again.${persistenceSuffix}`
      );
    } catch (error) {
      if (generation !== generationRef.current) return;
      showActionError(error);
      setExportStatus("Export failed. Choose Export .docx to try again.");
      // A failed local write is persisted even though the export command
      // returns an error. Refresh so that status is visible immediately.
      try {
        const latest = await loadReportWorkspace(clientId, visibleReportId);
        if (
          generation === generationRef.current &&
          latest.report_id === visibleReportId
        ) {
          setWorkspace(latest);
          setEdit(draftToEdit(latest.draft));
        }
      } catch {
        // Keep the actionable export error when status refresh is unavailable.
      }
    } finally {
      if (generation === generationRef.current) setBusy(null);
    }
  }, [clientId, showActionError]);

  const renameSession = useCallback(
    async (name: string) => {
      const current = stateRef.current.workspace;
      if (!current) return;
      const result = await renameReportSession(clientId, current.report_id, name);
      setWorkspace(result);
    },
    [clientId]
  );

  /** Adopt a workspace restored by the revision modal. */
  const applyReverted = useCallback((updated: ReportWorkspaceView) => {
    setWorkspace(updated);
    setEdit(draftToEdit(updated.draft));
    setEditing(false);
    setActionError(null);
    setConflict(false);
    setExportStatus(null);
    setSaveStatus(`Restored as revision ${updated.draft.revision}.`);
  }, []);

  return {
    workspace,
    loading,
    loadError,
    actionError,
    conflict,
    busy,
    editing,
    edit,
    setEdit,
    baseline,
    dirty,
    editCount,
    validationErrors,
    savedEditsQueued,
    saveStatus,
    setSaveStatus,
    exportStatus,
    agentActivity,
    liveContext,
    run,
    draftPane,
    planGate,
    // Every run command mints a stream, so any run this hook started can be
    // stopped — including one it picked back up.
    canStopRun:
      (busy === "generating" || busy === "planning") &&
      !run.stopping &&
      streamIdRef.current !== null,
    load,
    beginEdit,
    cancelEdit,
    save,
    discardQueuedEdits,
    send,
    // The legacy un-gated path. The writer surface plans instead, but the
    // wrapper and its command stay available.
    generateFullDraft,
    planRun,
    startPlannedRun,
    openResumeGate,
    stopRun,
    resumeRun,
    keepPartialDraft,
    discardRun,
    resolveProposal,
    applyTemplate,
    exportDocx,
    renameSession,
    applyReverted,
  };
}
