import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  applyReportTemplate,
  discardReportTemplatePreview,
  exportReportDocx,
  generateFullReport,
  loadReportWorkspace,
  previewWriterTemplate,
  renameReportSession,
  resolveReportProposal,
  saveReportDraft,
  sendReportMessage,
  type ReportDraftEdit,
  type ReportTurnProgressView,
  type ReportWorkspaceView,
} from "./tauri";
import {
  countReportEdits,
  draftToEdit,
  reportEditsEqual,
  validateReportEdit,
} from "./writingWorkspace";
import { upsertLiveContext, type ContextPill } from "./contextPills";

export type WriterBusy =
  | null
  | "saving"
  | "sending"
  | "generating"
  | "resolving"
  | "exporting"
  | "applying_template";

export type AgentActivity = { label: string; detail?: string } | null;

function isReportConflict(message: string): boolean {
  return message.toLowerCase().includes("changed on another computer");
}

function agentActivityForTool(name: string, context: string | null) {
  if (name === "list_record_files") {
    return { label: "Checking available records", detail: context ?? undefined };
  }
  if (name === "read_record_file") {
    return { label: "Reading client context", detail: context ?? undefined };
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
  const stateRef = useRef({ workspace, edit, dirty, busy, validationErrors });
  stateRef.current = { workspace, edit, dirty, busy, validationErrors };

  const showActionError = useCallback((error: unknown) => {
    const message = String(error);
    setActionError(message);
    setConflict(isReportConflict(message));
  }, []);

  const load = useCallback(async () => {
    const generation = ++generationRef.current;
    setLoading(true);
    setLoadError(null);
    setActionError(null);
    setConflict(false);
    try {
      const result = await loadReportWorkspace(clientId);
      if (generation !== generationRef.current) return;
      if (expectedReportId && result.report_id !== expectedReportId) {
        throw new Error("That Editor History session is no longer available.");
      }
      setWorkspace(result);
      setEdit(draftToEdit(result.draft));
      setEditing(false);
    } catch (error) {
      if (generation !== generationRef.current) return;
      setLoadError(String(error));
    } finally {
      if (generation === generationRef.current) setLoading(false);
    }
  }, [clientId, expectedReportId]);

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
        label:
          progress.call_number === 1 ? "Claude is planning" : "Claude is continuing",
        detail: `Model call ${progress.call_number}`,
      });
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
          current.draft.revision,
          modelId,
          instruction,
          references,
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

  const generateFullDraft = useCallback(
    async (modelId: string, guidance: string): Promise<boolean> => {
      const { busy: currentBusy } = stateRef.current;
      if (currentBusy !== null) return false;
      const generation = generationRef.current;
      setBusy("generating");
      setActionError(null);
      setLiveContext([]);
      setAgentActivity({
        label: "Preparing the complete report",
        detail: "Loading every readable client record",
      });
      setSaveStatus(
        stateRef.current.dirty
          ? "Saving your report edits before generating the complete draft…"
          : "Loading all readable records for the complete draft…"
      );
      try {
        const current = await persistCurrentEdit();
        if (generation !== generationRef.current) return false;
        const result = await generateFullReport(
          clientId,
          current.draft.revision,
          modelId,
          guidance,
          (progress) => {
            if (generation === generationRef.current) handleAgentProgress(progress);
          }
        );
        if (generation !== generationRef.current) return false;
        setWorkspace(result.workspace);
        setEdit(draftToEdit(result.workspace.draft));
        setEditing(false);
        setConflict(false);
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
        if (generation !== generationRef.current) return false;
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
        const result = await resolveReportProposal(clientId, proposalId, decision);
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
      setExportStatus(
        result.exported
          ? `Word document exported from revision ${result.revision}.${persistenceSuffix}`
          : `Export canceled. You can try again.${persistenceSuffix}`
      );
    } catch (error) {
      if (generation !== generationRef.current) return;
      showActionError(error);
      setExportStatus("Export failed. Choose Export .docx to try again.");
      // A failed local write is persisted even though the export command
      // returns an error. Refresh so that status is visible immediately.
      try {
        const latest = await loadReportWorkspace(clientId);
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
    load,
    beginEdit,
    cancelEdit,
    save,
    send,
    generateFullDraft,
    resolveProposal,
    applyTemplate,
    exportDocx,
    renameSession,
    applyReverted,
  };
}
