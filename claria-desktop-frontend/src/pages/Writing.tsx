import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  exportReportDocx,
  loadReportWorkspace,
  resolveReportProposal,
  saveReportDraft,
  sendReportMessage,
  type ChatModel,
  type ReportContextReadView,
  type ReportDraftEdit,
  type ReportTimelineItemView,
  type ReportWorkspaceView,
} from "../lib/tauri";
import {
  countReportEdits,
  draftToEdit,
  reportEditsEqual,
  validateReportEdit,
} from "../lib/writingWorkspace";
import WritingCanvas from "../components/WritingCanvas";
import WritingProposalCard from "../components/WritingProposalCard";
import Spinner from "../components/Spinner";
import { CloseIcon } from "../components/icons";
import { dismissNotice, isNoticeDismissed } from "../lib/localPreference";
import {
  readWritingComposerDraft,
  writeWritingComposerDraft,
  type WritingParagraphReference,
} from "../lib/writingComposerDraft";

const INTRO_NOTICE_KEY = "claria.writing.hide_intro_notice";

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
}: {
  clientId: string;
  expectedReportId?: string | null;
  chatModels: ChatModel[];
  chatModelsLoading: boolean;
  chatModelsError: string | null;
  preferredModelId?: string | null;
  onLeaveStateChange?: (state: WritingLeaveState) => void;
  onRetryModels?: () => void;
}) {
  const generationRef = useRef(0);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  const initialComposerDraft = useRef(readWritingComposerDraft(clientId)).current;
  const [workspace, setWorkspace] = useState<ReportWorkspaceView | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<
    null | "saving" | "sending" | "resolving" | "exporting"
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
  const [references, setReferences] = useState<WritingParagraphReference[]>(
    initialComposerDraft?.references ?? []
  );
  const [saveStatus, setSaveStatus] = useState<string | null>(null);
  const [exportStatus, setExportStatus] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [contextOpen, setContextOpen] = useState(false);
  const [showIntroNotice, setShowIntroNotice] = useState(
    () => !isNoticeDismissed(INTRO_NOTICE_KEY)
  );
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
        "Reloading will discard your local report edits. Your typed instruction and paragraph references will be kept. Continue?"
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

  async function handleSend() {
    const value = instruction.trim();
    if (!workspace || !value || composerDisabled) return;
    const generation = generationRef.current;
    setBusy("sending");
    setActionError(null);
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
        }))
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
    } catch (error) {
      if (generation !== generationRef.current) return;
      // Keep the instruction, references, and local edit for an exact retry.
      showActionError(error);
      setSaveStatus(null);
    } finally {
      if (generation === generationRef.current) setBusy(null);
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

  async function handleExport() {
    if (dirty || controlsBusy || !workspace) return;
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

  function addReference(reference: WritingParagraphReference) {
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
    setSaveStatus("Paragraph attached to your next Writing message.");
    requestAnimationFrame(() => composerRef.current?.focus());
  }

  function dismissIntroNotice() {
    dismissNotice(INTRO_NOTICE_KEY);
    setShowIntroNotice(false);
  }

  const contextReads = workspace.turns.flatMap((turn) => turn.context_reads);

  return (
    <div className="flex-1 min-h-0 grid grid-cols-1 min-[800px]:grid-cols-[minmax(340px,42%)_minmax(0,58%)] overflow-y-auto min-[800px]:overflow-hidden">
      <section className="min-h-[32rem] min-[800px]:min-h-0 flex flex-col bg-white">
        <div className="px-5 py-4 border-b border-gray-200 space-y-3">
          {showIntroNotice && (
            <div className="relative rounded-md border border-blue-200 bg-blue-50 p-3 pr-9">
              <p className="text-xs font-semibold text-blue-900">
                Writing assistant
              </p>
              <p className="text-xs leading-5 text-blue-800 mt-1">
                Claude can list and read bounded text from this client&apos;s record.
                It cannot change the report directly: every AI write is a proposal
                you must accept. Your own report edits are included automatically
                with your next message.
              </p>
              <button
                type="button"
                aria-label="Hide Writing assistant notice"
                title="Hide this notice"
                onClick={dismissIntroNotice}
                className="absolute right-2 top-2 text-blue-500 hover:text-blue-900"
              >
                <CloseIcon className="w-3.5 h-3.5" />
              </button>
            </div>
          )}

          <div className="grid grid-cols-[minmax(0,1fr)_auto] gap-2 items-end">
            <label className="block">
              <span className="text-xs font-medium text-gray-600">Model</span>
              <select
                aria-label="Writing model"
                value={selectedModelId}
                onChange={(event) => setSelectedModelId(event.target.value)}
                disabled={controlsBusy || pending !== null || chatModelsLoading}
                className="mt-1 w-full px-3 py-2 text-sm border border-gray-300 rounded-md bg-white focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50"
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
            </label>
            <button
              type="button"
              aria-expanded={contextOpen}
              aria-controls="writing-context-control"
              onClick={() => setContextOpen((open) => !open)}
              className="mb-px px-3 py-2 text-xs font-medium border border-gray-300 rounded-md bg-white hover:bg-gray-50"
            >
              Context · {contextReads.length + 1}
            </button>
          </div>

          {contextOpen && (
            <ContextControl
              revision={workspace.draft.revision}
              turns={workspace.turns.length}
              reads={contextReads}
              references={references}
            />
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
                Ask Claude to inspect records, answer a question, or propose
                specific sections.
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
            <div className="flex flex-wrap gap-1.5 mb-2" aria-label="Referenced report paragraphs">
              {references.map((reference) => (
                <span
                  key={`${reference.sectionId}-${reference.blockIndex}`}
                  className="inline-flex items-center gap-1.5 max-w-full px-2 py-1 text-[11px] text-blue-800 bg-blue-50 border border-blue-200 rounded-full"
                >
                  <span className="truncate">
                    {reference.sectionHeading} ¶{reference.blockIndex + 1}: {reference.preview}
                  </span>
                  <button
                    type="button"
                    aria-label={`Remove reference to ${reference.sectionHeading}, paragraph ${reference.blockIndex + 1}`}
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
        onReference={addReference}
        saveStatus={null}
        exportStatus={exportStatus}
        validationErrors={validationErrors}
      />
    </div>
  );
}

function ContextControl({
  revision,
  turns,
  reads,
  references,
}: {
  revision: number;
  turns: number;
  reads: ReportContextReadView[];
  references: WritingParagraphReference[];
}) {
  const uniqueReads = Array.from(
    new Map(
      reads.map((read) => [
        `${read.filename}:${read.offset}:${read.returned_characters}`,
        read,
      ])
    ).values()
  );
  return (
    <div
      id="writing-context-control"
      className="rounded-md border border-gray-200 bg-gray-50 p-3 text-xs text-gray-600 space-y-2"
    >
      <div>
        <p className="font-semibold text-gray-800">Included on the next message</p>
        <ul className="mt-1 list-disc pl-4 space-y-0.5">
          <li>Complete accepted report · revision {revision}</li>
          <li>{turns} retained Writing turn{turns === 1 ? "" : "s"}</li>
          {references.length > 0 && (
            <li>{references.length} paragraph reference{references.length === 1 ? "" : "s"}</li>
          )}
        </ul>
      </div>
      <div>
        <p className="font-semibold text-gray-800">
          Record excerpts read this session · {uniqueReads.length}
        </p>
        {uniqueReads.length === 0 ? (
          <p className="mt-1 text-gray-500">No record files have been read yet.</p>
        ) : (
          <ul className="mt-1 space-y-1 max-h-28 overflow-y-auto">
            {uniqueReads.map((read) => (
              <li key={`${read.filename}:${read.offset}:${read.returned_characters}`}>
                <span className="font-medium">{read.filename}</span>{" "}
                <span className="text-gray-500">
                  chars {read.offset}–{read.offset + read.returned_characters}
                </span>
              </li>
            ))}
          </ul>
        )}
        <p className="mt-1 text-[10px] leading-4 text-gray-400">
          Excerpt text is not stored in history. Claude re-reads source text on demand.
        </p>
      </div>
    </div>
  );
}

function reconcileReferences(
  references: WritingParagraphReference[],
  workspace: ReportWorkspaceView
): WritingParagraphReference[] {
  return references.flatMap((reference) => {
    const section = workspace.draft.content.sections.find(
      (candidate) => candidate.id === reference.sectionId
    );
    const block = section?.blocks[reference.blockIndex];
    if (!section || !block || block.kind !== "paragraph") return [];
    const compact = block.text.replace(/\s+/g, " ").trim();
    return [
      {
        ...reference,
        sectionHeading: section.heading,
        preview: compact.length > 90 ? `${compact.slice(0, 87)}…` : compact,
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
