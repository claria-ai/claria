import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  applyReportTemplate,
  discardReportTemplatePreview,
  exportReportDocx,
  listWriterTemplates,
  loadReportWorkspace,
  previewWriterTemplate,
  renameReportSession,
  resolveReportProposal,
  saveReportDraft,
  sendReportMessage,
  type ChatModel,
  type ReportDraftEdit,
  type ReportTimelineItemView,
  type ReportTurnProgressView,
  type ReportWorkspaceView,
  type WriterTemplateView,
} from "../lib/tauri";
import {
  countReportEdits,
  draftToEdit,
  reportEditsEqual,
  validateReportEdit,
} from "../lib/writingWorkspace";
import EditableName from "../components/EditableName";
import RecordFilePreviewModal from "../components/RecordFilePreviewModal";
import ReportRevisionModal from "../components/ReportRevisionModal";
import WritingCanvas from "../components/WritingCanvas";
import WritingProposalCard from "../components/WritingProposalCard";
import Spinner from "../components/Spinner";
import {
  readWritingComposerDraft,
  reportBlockReferencePreview,
  writeWritingComposerDraft,
  type WritingBlockReference,
} from "../lib/writingComposerDraft";

type ContextPill = {
  key: string;
  label: string;
  status: "loading" | "ready" | "failed";
  filename?: string;
};

export type WritingLeaveState = {
  /** Any work that would be lost when the desktop app closes. */
  hasUnsavedWork: boolean;
  /** Inline report changes that cannot be restored after leaving this page. */
  hasUnsavedReportEdits: boolean;
  busy: boolean;
};

export default function Writing({
  clientId,
  expectedReportId,
  chatModels,
  chatModelsLoading,
  chatModelsError,
  preferredModelId,
  onLeaveStateChange,
  onRetryModels,
  onManageTemplates,
}: {
  clientId: string;
  expectedReportId?: string | null;
  chatModels: ChatModel[];
  chatModelsLoading: boolean;
  chatModelsError: string | null;
  preferredModelId?: string | null;
  onLeaveStateChange?: (state: WritingLeaveState) => void;
  onRetryModels?: () => void;
  onManageTemplates: () => void;
}) {
  const generationRef = useRef(0);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  const initialComposerDraft = useRef(readWritingComposerDraft(clientId)).current;
  const [workspace, setWorkspace] = useState<ReportWorkspaceView | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<
    | null
    | "saving"
    | "sending"
    | "resolving"
    | "exporting"
    | "applying_template"
  >(null);
  const [selectedModelId, setSelectedModelId] = useState("");
  const [instruction, setInstruction] = useState(
    initialComposerDraft?.instruction ?? ""
  );
  const [editing, setEditing] = useState(false);
  const [edit, setEdit] = useState<ReportDraftEdit>({
    title: "Untitled report",
    sections: [],
  });
  const [references, setReferences] = useState<WritingBlockReference[]>(
    initialComposerDraft?.references ?? []
  );
  const [saveStatus, setSaveStatus] = useState<string | null>(null);
  const [exportStatus, setExportStatus] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [contextOpen, setContextOpen] = useState(false);
  const [previewFilename, setPreviewFilename] = useState<string | null>(null);
  const [revisionsOpen, setRevisionsOpen] = useState(false);
  const [agentActivity, setAgentActivity] = useState<{
    label: string;
    detail?: string;
  } | null>(null);
  const [liveContext, setLiveContext] = useState<ContextPill[]>([]);
  const [writerTemplates, setWriterTemplates] = useState<WriterTemplateView[]>([]);
  const [selectedTemplateId, setSelectedTemplateId] = useState("");
  const [templatesError, setTemplatesError] = useState<string | null>(null);
  const timelineEndRef = useRef<HTMLDivElement | null>(null);
  const proposalStartRef = useRef<HTMLDivElement | null>(null);

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
      setReferences((current) => reconcileReferences(current, result));
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
  }, [clientId, expectedReportId, load]);

  const loadTemplates = useCallback(async () => {
    try {
      const templates = await listWriterTemplates();
      setWriterTemplates(templates);
      setSelectedTemplateId((current) =>
        templates.some((template) => template.id === current)
          ? current
          : (templates[0]?.id ?? "")
      );
      setTemplatesError(null);
    } catch (error) {
      setTemplatesError(String(error));
    }
  }, []);

  useEffect(() => {
    void loadTemplates();
  }, [loadTemplates]);

  useEffect(() => {
    writeWritingComposerDraft(clientId, { instruction, references });
  }, [clientId, instruction, references]);

  useEffect(() => {
    if (
      selectedModelId &&
      chatModels.some((model) => model.model_id === selectedModelId)
    ) {
      return;
    }
    const preferred = chatModels.find(
      (model) => model.model_id === preferredModelId
    );
    setSelectedModelId(preferred?.model_id ?? chatModels[0]?.model_id ?? "");
  }, [chatModels, preferredModelId, selectedModelId]);

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
  const editsQueued = dirty || savedEditsQueued;
  const hasUnsavedWork =
    dirty || instruction.trim() !== "" || references.length > 0;

  useEffect(() => {
    onLeaveStateChange?.({
      hasUnsavedWork,
      hasUnsavedReportEdits: dirty,
      busy: busy !== null,
    });
    return () =>
      onLeaveStateChange?.({
        hasUnsavedWork: false,
        hasUnsavedReportEdits: false,
        busy: false,
      });
  }, [busy, dirty, hasUnsavedWork, onLeaveStateChange]);

  const pendingProposalId = workspace?.pending_proposal?.id;
  useEffect(() => {
    if (pendingProposalId) {
      proposalStartRef.current?.scrollIntoView?.({ block: "start" });
    } else {
      timelineEndRef.current?.scrollIntoView?.({ block: "nearest" });
    }
  }, [workspace?.turns.length, pendingProposalId]);

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center bg-gray-50">
        <div role="status" className="flex items-center gap-2 text-sm text-gray-500">
          <Spinner />
          <span>Loading writing session…</span>
        </div>
      </div>
    );
  }

  if (loadError || !workspace) {
    return (
      <div className="flex-1 flex items-center justify-center bg-gray-50 p-8">
        <div className="max-w-md border border-red-200 bg-white rounded-lg p-6 text-center">
          <h3 className="text-sm font-semibold text-gray-900">
            Could not load the writing session
          </h3>
          <p role="alert" className="text-sm text-red-600 mt-2">
            {loadError ?? "Unknown writing session error"}
          </p>
          <button
            type="button"
            onClick={() => void load()}
            className="mt-4 px-4 py-2 text-sm text-white bg-blue-600 rounded-md hover:bg-blue-700"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  const pending = workspace.pending_proposal;
  const controlsBusy = busy !== null;
  const composerDisabled =
    controlsBusy ||
    pending !== null ||
    !selectedModelId ||
    chatModelsLoading ||
    (dirty && validationErrors.length > 0);

  function showActionError(error: unknown) {
    const message = String(error);
    setActionError(message);
    setConflict(isReportConflict(message));
  }

  async function handleReload() {
    if (controlsBusy) return;
    if (
      dirty &&
      !window.confirm(
        "Reloading will discard your local report edits. Your typed instruction and report references will be kept. Continue?"
      )
    ) {
      return;
    }
    await load();
  }

  async function persistCurrentEdit(): Promise<ReportWorkspaceView> {
    if (!workspace) throw new Error("The writing session is not loaded.");
    if (!dirty) return workspace;
    if (validationErrors.length > 0) {
      throw new Error("Fix the highlighted report fields before continuing.");
    }
    const result = await saveReportDraft(
      clientId,
      workspace.draft.revision,
      edit
    );
    setWorkspace(result);
    setEdit(draftToEdit(result.draft));
    setConflict(false);
    return result;
  }

  async function handleSave() {
    if (!dirty || controlsBusy || validationErrors.length > 0) return;
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
  }

  function handleAgentProgress(progress: ReportTurnProgressView) {
    if (progress.kind === "model_call_started") {
      setAgentActivity({
        label: progress.call_number === 1 ? "Claude is planning" : "Claude is continuing",
        detail: `Model call ${progress.call_number}`,
      });
      return;
    }

    const context = progress.context;
    if (progress.kind === "tool_started") {
      setAgentActivity(agentActivityForTool(progress.name, context));
      if (context) {
        setLiveContext((current) => upsertLiveContext(current, context, "loading"));
      }
      return;
    }

    setAgentActivity(
      progress.name === "propose_report_changes"
        ? { label: "Claude is reviewing its proposal" }
        : {
            label: progress.status === "succeeded" ? "Context ready" : "Context unavailable",
            detail: context ?? progress.name,
          }
    );
    if (context) {
      setLiveContext((current) =>
        upsertLiveContext(
          current,
          context,
          progress.status === "succeeded" ? "ready" : "failed"
        )
      );
    }
  }

  async function handleSend() {
    const value = instruction.trim();
    if (!workspace || !value || composerDisabled) return;
    const generation = generationRef.current;
    setBusy("sending");
    setActionError(null);
    setLiveContext([]);
    setAgentActivity({ label: "Preparing the writer", detail: "Loading approved context" });
    setSaveStatus(
      dirty
        ? "Saving your report edits before Claude reads the next message…"
        : "Claude is using the approved tools…"
    );
    try {
      const current = await persistCurrentEdit();
      if (generation !== generationRef.current) return;
      const result = await sendReportMessage(
        clientId,
        current.draft.revision,
        selectedModelId,
        value,
        references.map((reference) => ({
          section_id: reference.sectionId,
          block_index: reference.blockIndex,
        })),
        (progress) => {
          if (generation === generationRef.current) handleAgentProgress(progress);
        }
      );
      if (generation !== generationRef.current) return;
      setWorkspace(result.workspace);
      setEdit(draftToEdit(result.workspace.draft));
      setInstruction("");
      setReferences([]);
      setConflict(false);
      setSaveStatus(
        result.workspace.pending_proposal
          ? "Proposal ready for your review. The accepted draft is unchanged."
          : "Writing assistant turn complete."
      );
      setLiveContext([]);
    } catch (error) {
      if (generation !== generationRef.current) return;
      // Keep the instruction, references, and local edit for an exact retry.
      showActionError(error);
      setSaveStatus(null);
      setLiveContext([]);
    } finally {
      if (generation === generationRef.current) {
        setAgentActivity(null);
        setBusy(null);
      }
    }
  }

  async function handleDecision(decision: "accept" | "reject") {
    if (!workspace?.pending_proposal || controlsBusy) return;
    const generation = generationRef.current;
    const proposalId = workspace.pending_proposal.id;
    setBusy("resolving");
    setActionError(null);
    setSaveStatus(
      decision === "accept" ? "Accepting and saving proposal…" : "Rejecting proposal…"
    );
    try {
      const result = await resolveReportProposal(
        clientId,
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
  }

  async function handleApplyTemplate() {
    const currentWorkspace = workspace;
    if (
      !currentWorkspace ||
      !selectedTemplateId ||
      controlsBusy ||
      dirty ||
      editing ||
      currentWorkspace.pending_proposal
    ) {
      return;
    }
    const generation = generationRef.current;
    let importId: string | null = null;
    setBusy("applying_template");
    setActionError(null);
    setSaveStatus("Applying the managed Word template…");
    try {
      const preview = await previewWriterTemplate(clientId, selectedTemplateId);
      importId = preview.import_id;
      if (generation !== generationRef.current) return;
      const result = await applyReportTemplate(
        clientId,
        currentWorkspace.draft.revision,
        preview.import_id
      );
      if (generation !== generationRef.current) return;
      setWorkspace(result);
      setEdit(draftToEdit(result.draft));
      setEditing(false);
      setConflict(false);
      setSaveStatus(
        `Applied the Word template as revision ${result.draft.revision}. Its layout and formatting will be retained on export.`
      );
      void loadTemplates();
    } catch (error) {
      if (generation !== generationRef.current) return;
      showActionError(error);
      setSaveStatus(null);
    } finally {
      if (importId) {
        await discardReportTemplatePreview(importId).catch(() => undefined);
      }
      if (generation === generationRef.current) setBusy(null);
    }
  }

  async function handleExport() {
    if (dirty || controlsBusy || !workspace) {
      return;
    }
    const generation = generationRef.current;
    const visibleReportId = workspace.report_id;
    const visibleRevision = workspace.draft.revision;
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
      setWorkspace((current) =>
        current
          ? {
              ...current,
              last_export: {
                revision: result.revision,
                status: result.status,
                attempted_at: result.attempted_at,
              },
            }
          : current
      );
      setConflict(false);
      const persistenceSuffix = result.status_persisted
        ? ""
        : " Export status could not be synced to Editor History."
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
  }

  function addReference(reference: WritingBlockReference) {
    setReferences((current) => {
      if (
        current.some(
          (item) =>
            item.sectionId === reference.sectionId &&
            item.blockIndex === reference.blockIndex
        )
      ) {
        return current;
      }
      return [...current, reference].slice(-10);
    });
    setSaveStatus(
      `${reference.kind === "paragraph" ? "Paragraph" : "Table"} attached to your next Writing message.`
    );
    requestAnimationFrame(() => composerRef.current?.focus());
  }

  async function handleRenameSession(name: string) {
    const currentWorkspace = workspace;
    if (!currentWorkspace) return;
    const result = await renameReportSession(
      clientId,
      currentWorkspace.report_id,
      name
    );
    setWorkspace(result);
  }

  const contextReads = workspace.turns.flatMap((turn) => turn.context_reads);
  const contextPills: ContextPill[] = [
    {
      key: "accepted-report",
      label: `Accepted report · r${workspace.draft.revision}`,
      status: "ready",
    },
  ];
  if (workspace.turns.length > 0) {
    contextPills.push({
      key: "session-history",
      label: `${workspace.turns.length} prior turn${workspace.turns.length === 1 ? "" : "s"}`,
      status: "ready",
    });
  }
  if (workspace.template_import) {
    contextPills.push({ key: "template", label: "Template provenance", status: "ready" });
  }
  if (
    workspace.turns.some((turn) =>
      turn.timeline.some(
        (item) => item.kind === "tool_activity" && item.name === "list_record_files"
      )
    )
  ) {
    contextPills.push({ key: "record-list", label: "Record file list", status: "ready" });
  }
  for (const filename of new Set(contextReads.map((read) => read.filename))) {
    contextPills.push({
      key: `record:${filename}`,
      label: filename,
      status: "ready",
      filename,
    });
  }
  for (const reference of references) {
    contextPills.push({
      key: `reference:${reference.sectionId}:${reference.blockIndex}`,
      label: `${reference.sectionHeading} · ${reference.kind}`,
      status: "ready",
    });
  }
  for (const live of liveContext) {
    const existing = contextPills.find((pill) => pill.label === live.label);
    if (existing) {
      existing.status = live.status;
      existing.filename ??= live.filename;
    } else contextPills.push(live);
  }

  return (
    <>
      <div className="flex-1 min-h-0 grid grid-cols-1 min-[800px]:grid-cols-[minmax(340px,42%)_minmax(0,58%)] overflow-y-auto min-[800px]:overflow-hidden">
      <section className="min-h-[32rem] min-[800px]:min-h-0 flex flex-col bg-white">
        <div className="px-5 py-4 border-b border-gray-200 space-y-3">
          <div className="grid grid-cols-[minmax(4rem,0.7fr)_minmax(7rem,1fr)_auto] items-center gap-2">
            <div className="min-w-0">
              <EditableName
                value={workspace.session_name}
                label="writer session"
                onSave={handleRenameSession}
                disabled={controlsBusy}
                compactActions
                className="w-full text-sm"
              />
            </div>
            <select
              aria-label="Writing model"
              value={selectedModelId}
              onChange={(event) => setSelectedModelId(event.target.value)}
              disabled={controlsBusy || pending !== null || chatModelsLoading}
              className="min-w-0 w-full px-2 py-1.5 text-sm border border-gray-300 rounded-md bg-white focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50"
            >
              {chatModelsLoading ? (
                <option value="">Loading models…</option>
              ) : chatModels.length === 0 ? (
                <option value="">No models available</option>
              ) : null}
              {chatModels.map((model) => (
                <option key={model.model_id} value={model.model_id}>
                  {model.name}
                </option>
              ))}
            </select>
            <button
              type="button"
              aria-expanded={contextOpen}
              aria-controls="writing-context-control"
              onClick={() => setContextOpen((open) => !open)}
              className="px-3 py-1.5 text-xs font-medium border border-gray-300 rounded-md bg-white hover:bg-gray-50"
            >
              Context · {contextPills.length}
            </button>
          </div>

          {contextOpen && (
            <div
              id="writing-context-control"
              className="rounded-md border border-gray-200 bg-gray-50 p-2.5"
            >
              <ContextPills
                pills={contextPills}
                onPreviewFile={setPreviewFilename}
              />
            </div>
          )}

          {workspace.template_import ? (
            <div
              title="Start a new Writing session to use another template"
              className="flex items-center gap-2 rounded-md border border-blue-100 bg-blue-50 px-3 py-2 text-xs text-blue-800"
            >
              <span aria-hidden="true">✓</span>
              <span className="min-w-0 truncate">
                Template <strong>{workspace.template_import.writer_template_name ?? "Word template"}</strong> applied
              </span>
            </div>
          ) : (
            <>
              <div className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-end gap-2">
                <label className="block min-w-0">
                  <span className="text-xs font-medium text-gray-600">Writer template</span>
                  <select
                    aria-label="Writer template"
                    value={selectedTemplateId}
                    onChange={(event) => setSelectedTemplateId(event.target.value)}
                    disabled={controlsBusy || pending !== null || writerTemplates.length === 0}
                    className="mt-1 w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50"
                  >
                    {writerTemplates.length === 0 && (
                      <option value="">No saved templates</option>
                    )}
                    {writerTemplates.map((template) => (
                      <option key={template.id} value={template.id}>
                        {template.name}
                      </option>
                    ))}
                  </select>
                </label>
                <button
                  type="button"
                  onClick={() => void handleApplyTemplate()}
                  disabled={
                    controlsBusy ||
                    pending !== null ||
                    dirty ||
                    editing ||
                    selectedTemplateId === ""
                  }
                  className="mb-px rounded-md border border-gray-300 bg-white px-3 py-2 text-xs font-medium hover:bg-gray-50 disabled:opacity-50"
                >
                  {busy === "applying_template" ? "Applying…" : "Apply template"}
                </button>
                <button
                  type="button"
                  onClick={onManageTemplates}
                  disabled={controlsBusy}
                  className="mb-px px-2 py-2 text-xs font-medium text-blue-700 hover:text-blue-900 disabled:opacity-50"
                >
                  Manage in Preferences
                </button>
              </div>
              {templatesError && (
                <p role="alert" className="text-xs text-red-600">
                  Could not load writer templates: {templatesError}
                </p>
              )}
            </>
          )}

          {chatModelsError && (
            <div className="flex items-center gap-2">
              <p role="alert" className="flex-1 text-xs text-red-600">
                Could not load models: {chatModelsError}
              </p>
              {onRetryModels && (
                <button
                  type="button"
                  onClick={onRetryModels}
                  className="text-xs font-medium text-blue-700 hover:text-blue-900"
                >
                  Retry
                </button>
              )}
            </div>
          )}
        </div>

        <div
          aria-label="Writing timeline"
          className="flex-1 overflow-y-auto px-5 py-4 space-y-4 select-text"
        >
          {workspace.turns.length === 0 && !pending && (
            <div className="py-8 text-center">
              <p className="text-sm font-medium text-gray-700">
                Build the report interactively.
              </p>
              <p className="text-xs text-gray-500 mt-1">
                Ask Claude to inspect records, answer a question, propose
                specific sections, or apply a managed Word template.
              </p>
            </div>
          )}
          {workspace.turns.map((turn) => (
            <div key={turn.id} className="space-y-2">
              {turn.timeline.map((item, index) => (
                <TimelineItem key={`${turn.id}-${index}`} item={item} />
              ))}
              <p className="text-[10px] text-gray-400 text-right">
                {turn.tool_uses} tool use{turn.tool_uses === 1 ? "" : "s"} ·{" "}
                {turn.usage.input_tokens + turn.usage.output_tokens} tokens
              </p>
            </div>
          ))}

          {pending && (
            <div ref={proposalStartRef}>
              <WritingProposalCard
                proposal={pending}
                accepted={workspace.draft.content}
                busy={busy === "resolving"}
                onAccept={() => void handleDecision("accept")}
                onReject={() => void handleDecision("reject")}
              />
            </div>
          )}
          <div ref={timelineEndRef} />
        </div>

        {actionError && (
          <div
            role="alert"
            className="mx-5 mb-2 p-3 text-xs text-red-700 bg-red-50 border border-red-200 rounded-md"
          >
            <p>{actionError}</p>
            {conflict && (
              <button
                type="button"
                onClick={() => void handleReload()}
                className="mt-2 font-semibold text-blue-700 hover:text-blue-900"
              >
                Reload writing session
              </button>
            )}
          </div>
        )}
        {saveStatus && (
          <p
            role="status"
            aria-live="polite"
            className="px-5 pb-2 text-xs text-gray-600"
          >
            {saveStatus}
          </p>
        )}

        <div className="border-t border-gray-200 p-4">
          {editsQueued && (
            <p className="text-xs text-amber-700 mb-2" data-testid="queued-report-edits">
              {dirty
                ? `${editCount} report edit${editCount === 1 ? "" : "s"} queued. Claria will save and include them with your next message.`
                : "Saved report edits are queued and will be included with your next message."}
            </p>
          )}
          {pending && (
            <p className="text-xs text-violet-700 mb-2">
              Accept or reject the proposal before continuing.
            </p>
          )}
          {references.length > 0 && (
            <div
              className="flex flex-wrap gap-1.5 mb-2"
              aria-label="Referenced report blocks"
            >
              {references.map((reference) => (
                <span
                  key={`${reference.sectionId}-${reference.blockIndex}`}
                  className="inline-flex items-center gap-1.5 max-w-full px-2 py-1 text-[11px] text-blue-800 bg-blue-50 border border-blue-200 rounded-full"
                >
                  <span className="truncate">
                    {reference.sectionHeading}{" "}
                    {reference.kind === "paragraph"
                      ? `¶${reference.blockIndex + 1}`
                      : `table ${reference.blockIndex + 1}`}
                    : {reference.preview}
                  </span>
                  <button
                    type="button"
                    aria-label={`Remove reference to ${reference.sectionHeading}, ${reference.kind} ${reference.blockIndex + 1}`}
                    onClick={() =>
                      setReferences((current) =>
                        current.filter(
                          (item) =>
                            item.sectionId !== reference.sectionId ||
                            item.blockIndex !== reference.blockIndex
                        )
                      )
                    }
                    className="shrink-0 text-blue-500 hover:text-blue-900"
                  >
                    ×
                  </button>
                </span>
              ))}
            </div>
          )}
          <textarea
            ref={composerRef}
            aria-label="Writing instruction"
            value={instruction}
            onChange={(event) => setInstruction(event.target.value)}
            onKeyDown={(event) => {
              if (
                event.key === "Enter" &&
                (event.ctrlKey || event.metaKey) &&
                !composerDisabled
              ) {
                event.preventDefault();
                void handleSend();
              }
            }}
            disabled={composerDisabled}
            rows={4}
            placeholder="Ask a question or describe the report change you want…"
            className="w-full px-3 py-2 text-sm border border-gray-300 rounded-md resize-y focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50 disabled:text-gray-500"
          />
          <div className="mt-2 flex items-center justify-between">
            <span className="text-[11px] text-gray-400">Ctrl/Cmd + Enter to send</span>
            <button
              type="button"
              onClick={() => void handleSend()}
              disabled={composerDisabled || instruction.trim() === ""}
              className="px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50"
            >
              {busy === "sending" ? "Using tools…" : "Send"}
            </button>
          </div>
        </div>
      </section>

      <WritingCanvas
        workspace={workspace}
        edit={edit}
        editing={editing}
        dirty={dirty}
        busy={controlsBusy}
        onBeginEdit={() => {
          setEdit(draftToEdit(workspace.draft));
          setEditing(true);
          setActionError(null);
          setSaveStatus(null);
        }}
        onCancelEdit={() => {
          setEdit(draftToEdit(workspace.draft));
          setEditing(false);
          setActionError(null);
        }}
        onChange={setEdit}
        onSave={() => void handleSave()}
        onExport={() => void handleExport()}
        onOpenRevisions={() => setRevisionsOpen(true)}
        onReference={addReference}
        saveStatus={null}
        exportStatus={exportStatus}
        validationErrors={validationErrors}
        agentActivity={agentActivity}
      />
      </div>

      {previewFilename && (
        <RecordFilePreviewModal
          clientId={clientId}
          filename={previewFilename}
          onClose={() => setPreviewFilename(null)}
        />
      )}
      {revisionsOpen && (
        <ReportRevisionModal
          clientId={clientId}
          workspace={workspace}
          canRevert={
            !controlsBusy && !dirty && !editing && pending === null
          }
          onClose={() => setRevisionsOpen(false)}
          onReverted={(updated) => {
            setWorkspace(updated);
            setEdit(draftToEdit(updated.draft));
            setEditing(false);
            setReferences([]);
            setActionError(null);
            setConflict(false);
            setExportStatus(null);
            setSaveStatus(`Restored as revision ${updated.draft.revision}.`);
          }}
        />
      )}
    </>
  );
}

function ContextPills({
  pills,
  onPreviewFile,
}: {
  pills: ContextPill[];
  onPreviewFile: (filename: string) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1.5" aria-label="Writer context">
      {pills.map((pill) => {
        const className = `inline-flex max-w-full items-center gap-1 rounded-full border px-2 py-1 text-[10px] font-medium ${
          pill.status === "failed"
            ? "border-red-200 bg-red-50 text-red-700"
            : pill.status === "loading"
              ? "border-blue-200 bg-blue-50 text-blue-700"
              : "border-emerald-200 bg-emerald-50 text-emerald-700"
        }`;
        const content = (
          <>
            <span
              aria-hidden="true"
              className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                pill.status === "failed"
                  ? "bg-red-500"
                  : pill.status === "loading"
                    ? "bg-blue-500 animate-pulse"
                    : "bg-emerald-500"
              }`}
            />
            <span className="truncate">{pill.label}</span>
          </>
        );
        return pill.filename ? (
          <button
            type="button"
            key={pill.key}
            onClick={() => onPreviewFile(pill.filename!)}
            title={`Preview ${pill.filename}`}
            className={`${className} hover:border-emerald-400 hover:text-emerald-900`}
          >
            {content}
          </button>
        ) : (
          <span key={pill.key} className={className}>
            {content}
          </span>
        );
      })}
    </div>
  );
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
  return { label: "Using an approved tool", detail: name };
}

function upsertLiveContext(
  current: ContextPill[],
  label: string,
  status: "loading" | "ready" | "failed"
) {
  const key = `live:${label}`;
  const existing = current.find((item) => item.key === key);
  if (existing) {
    return current.map((item) => (item.key === key ? { ...item, status } : item));
  }
  return [...current, { key, label, status, filename: label }];
}

function reconcileReferences(
  references: WritingBlockReference[],
  workspace: ReportWorkspaceView
): WritingBlockReference[] {
  return references.flatMap((reference) => {
    const section = workspace.draft.content.sections.find(
      (candidate) => candidate.id === reference.sectionId
    );
    const block = section?.blocks[reference.blockIndex];
    if (!section || !block || block.kind === "bullet_list") return [];
    if (block.kind !== reference.kind) return [];
    return [
      {
        ...reference,
        kind: block.kind,
        sectionHeading: section.heading,
        preview: reportBlockReferencePreview(block),
      },
    ];
  });
}

function isReportConflict(message: string): boolean {
  return message.toLowerCase().includes("changed on another computer");
}

function TimelineItem({ item }: { item: ReportTimelineItemView }) {
  if (item.kind === "tool_activity") {
    const color =
      item.status === "failed"
        ? "border-red-200 bg-red-50 text-red-700"
        : item.status === "succeeded"
          ? "border-emerald-200 bg-emerald-50 text-emerald-700"
          : "border-gray-200 bg-gray-50 text-gray-600";
    return (
      <details className={`group border rounded-md text-xs ${color}`}>
        <summary className="list-none cursor-pointer px-3 py-2 flex items-center gap-2 [&::-webkit-details-marker]:hidden">
          <span
            aria-hidden="true"
            className="inline-block transition-transform group-open:rotate-90"
          >
            ▸
          </span>
          <span className="flex-1">{item.summary}</span>
          <code className="hidden sm:inline text-[10px] opacity-70">
            {item.name}
          </code>
          <span className="capitalize">{item.status}</span>
        </summary>
        <div className="border-t border-current/15 bg-gray-950 text-gray-100 px-3 py-3 space-y-3 select-text">
          <div>
            <p className="mb-1 text-[10px] uppercase tracking-wide text-gray-400">
              Raw LLM invocation
            </p>
            <pre className="max-h-72 overflow-auto whitespace-pre text-[11px] leading-4 font-mono">
              {item.invocation_json}
            </pre>
          </div>
          {item.result_json && (
            <div>
              <p className="mb-1 text-[10px] uppercase tracking-wide text-gray-400">
                Correlated tool result
              </p>
              <pre className="max-h-72 overflow-auto whitespace-pre text-[11px] leading-4 font-mono">
                {item.result_json}
              </pre>
            </div>
          )}
        </div>
      </details>
    );
  }

  const user = item.role === "user";
  return (
    <div className={`flex ${user ? "justify-end" : "justify-start"}`}>
      <div
        className={`max-w-[90%] rounded-lg px-3 py-2 text-sm select-text ${
          user
            ? "bg-blue-600 text-white whitespace-pre-wrap"
            : "bg-gray-100 text-gray-800 border border-gray-200"
        }`}
      >
        {user ? (
          item.text
        ) : (
          <div className="prose prose-sm max-w-none prose-p:my-1 prose-ul:my-1 prose-ol:my-1 prose-li:my-0.5 prose-headings:my-2 prose-code:text-inherit prose-code:before:content-none prose-code:after:content-none">
            <Markdown remarkPlugins={[remarkGfm]}>{item.text}</Markdown>
          </div>
        )}
      </div>
    </div>
  );
}
