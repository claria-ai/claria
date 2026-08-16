import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useChatModels } from "../lib/chatModels";
import ChatComposer from "../components/ChatComposer";
import ChatEmptyState from "../components/ChatEmptyState";
import ContextPills from "../components/ContextPills";
import { buildContextPills } from "../lib/contextPills";
import EditableName from "../components/EditableName";
import ModelSelect from "../components/ModelSelect";
import Modal from "../components/Modal";
import RecordFilePreviewModal from "../components/RecordFilePreviewModal";
import ReportRevisionModal from "../components/ReportRevisionModal";
import SessionTabs, { UsageTabIcon } from "../components/SessionTabs";
import SessionUsagePanel from "../components/SessionUsagePanel";
import TimelineItem from "../components/TimelineItem";
import TurnCostBadge from "../components/TurnCostBadge";
import WritingCanvas from "../components/WritingCanvas";
import WritingProposalCard from "../components/WritingProposalCard";
import Spinner from "../components/Spinner";
import {
  EMPTY_SESSION_USAGE,
  accumulateUsage,
  type SessionUsage,
} from "../lib/cost";
import { buildCostLedger } from "../lib/costLedger";
import { buildWriterSessionDiagram } from "../lib/sessionDiagram";
import { usePreferredModel } from "../lib/usePreferredModel";
import { usePricingMap } from "../lib/usePricingMap";
import { useReportWorkspace } from "../lib/useReportWorkspace";
import { useWriterPrompts } from "../lib/useWriterPrompts";
import { useWriterTemplates } from "../lib/useWriterTemplates";
import WriterPromptPicker from "../components/WriterPromptPicker";
import {
  readWritingComposerDraft,
  reportBlockReferencePreview,
  writeWritingComposerDraft,
  type WritingBlockReference,
} from "../lib/writingComposerDraft";
import type { ReportWorkspaceView } from "../lib/tauri";

export type WritingLeaveState = {
  /** Any work that would be lost when the desktop app closes. */
  hasUnsavedWork: boolean;
  /** Inline report changes that cannot be restored after leaving this page. */
  hasUnsavedReportEdits: boolean;
  busy: boolean;
  /** A drafting run is generating right now, and can be stopped safely. */
  draftRunLive: boolean;
};

export default function Writing({
  clientId,
  expectedReportId,
  onLeaveStateChange,
  onManageTemplates,
  onManagePrompts,
}: {
  clientId: string;
  expectedReportId?: string | null;
  onLeaveStateChange?: (state: WritingLeaveState) => void;
  onManageTemplates: () => void;
  onManagePrompts: () => void;
}) {
  const {
    models: chatModels,
    loading: chatModelsLoading,
    error: chatModelsError,
    preferredModelId,
    retry: onRetryModels,
  } = useChatModels();
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  const [selectedModelId, setSelectedModelId] = usePreferredModel(
    chatModels,
    preferredModelId
  );
  // The composer draft survives navigation in process memory; read it once
  // as lazy initial state.
  const [instruction, setInstruction] = useState(() =>
    expectedReportId
      ? (readWritingComposerDraft(clientId, expectedReportId)?.instruction ?? "")
      : ""
  );
  const [queuedReferences, setQueuedReferences] = useState<
    WritingBlockReference[]
  >(() =>
    expectedReportId
      ? (readWritingComposerDraft(clientId, expectedReportId)?.references ?? [])
      : []
  );
  const [contextOpen, setContextOpen] = useState(false);
  const [previewFilename, setPreviewFilename] = useState<string | null>(null);
  const [revisionsOpen, setRevisionsOpen] = useState(false);
  const [fullDraftConfirmationOpen, setFullDraftConfirmationOpen] =
    useState(false);
  const [chosenTemplateId, setChosenTemplateId] = useState("");
  const [activePane, setActivePane] = useState<"setup" | "write" | "usage">(
    expectedReportId ? "write" : "setup"
  );
  const [showTurnCosts, setShowTurnCosts] = useState(false);
  const timelineEndRef = useRef<HTMLDivElement | null>(null);
  const proposalStartRef = useRef<HTMLDivElement | null>(null);

  const {
    workspace,
    loading,
    loadError,
    actionError,
    conflict,
    busy,
    editing,
    edit,
    setEdit,
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
    canStopRun,
    load,
    beginEdit,
    cancelEdit,
    save,
    discardQueuedEdits,
    send,
    generateFullDraft,
    stopRun,
    resumeRun,
    keepPartialDraft,
    discardRun,
    resolveProposal,
    applyTemplate,
    exportDocx,
    renameSession,
    applyReverted,
  } = useReportWorkspace({ clientId, expectedReportId });

  const {
    templates: writerTemplates,
    error: templatesError,
    reload: reloadTemplates,
  } = useWriterTemplates();

  const { prompts: writerPrompts } = useWriterPrompts();

  // Picking a saved prompt is a prefill, not a send: the body lands in the
  // instruction box for editing (e.g. replacing a $DIAGNOSIS placeholder).
  function insertSavedPrompt(body: string) {
    setInstruction(body);
    requestAnimationFrame(() => composerRef.current?.focus());
  }

  // The user's explicit template choice, falling back to the first template
  // while the choice is unset or no longer exists.
  const selectedTemplateId = writerTemplates.some(
    (template) => template.id === chosenTemplateId
  )
    ? chosenTemplateId
    : (writerTemplates[0]?.id ?? "");

  // Queued block references reconciled against the loaded draft: a reference
  // to a block that no longer exists (or changed kind) is dropped, and
  // headings/previews follow the current content.
  const references = useMemo(
    () =>
      workspace
        ? reconcileReferences(queuedReferences, workspace)
        : queuedReferences,
    [queuedReferences, workspace]
  );

  useEffect(() => {
    if (!workspace) return;
    writeWritingComposerDraft(clientId, workspace.report_id, {
      instruction,
      references,
    });
  }, [clientId, instruction, references, workspace]);

  const editsQueued = dirty || savedEditsQueued;
  const hasUnsavedWork =
    dirty || instruction.trim() !== "" || references.length > 0;

  const draftRunLive = run.live;
  useEffect(() => {
    onLeaveStateChange?.({
      hasUnsavedWork,
      hasUnsavedReportEdits: dirty,
      busy: busy !== null,
      draftRunLive,
    });
    return () =>
      onLeaveStateChange?.({
        hasUnsavedWork: false,
        hasUnsavedReportEdits: false,
        busy: false,
        draftRunLive: false,
      });
  }, [busy, dirty, draftRunLive, hasUnsavedWork, onLeaveStateChange]);

  const pendingProposalId = workspace?.pending_proposal?.id;
  useEffect(() => {
    if (pendingProposalId) {
      proposalStartRef.current?.scrollIntoView?.({ block: "start" });
    } else {
      timelineEndRef.current?.scrollIntoView?.({ block: "nearest" });
    }
  }, [workspace?.turns.length, pendingProposalId]);

  // Lifetime writer-session spend, matching the chat surfaces' banner.
  const session: SessionUsage = useMemo(
    () =>
      (workspace?.turns ?? []).reduce(
        (acc, turn) => accumulateUsage(acc, turn.usage),
        EMPTY_SESSION_USAGE
      ),
    [workspace?.turns]
  );

  // Cache-aware ledger over the same turn stream, for the savings line and
  // the expandable cost-explanation panel.
  const turnUsages = useMemo(
    () => (workspace?.turns ?? []).map((turn) => turn.usage),
    [workspace?.turns]
  );
  const turnTimestamps = useMemo(
    () => (workspace?.turns ?? []).map((turn) => turn.completed_at),
    [workspace?.turns]
  );
  const turnModelIds = useMemo(
    () => turnUsages.flatMap((usage) => (usage ? [usage.model_id] : [])),
    [turnUsages]
  );
  const pricingByModel = usePricingMap(
    activePane === "usage" ? turnModelIds : []
  );
  const ledger = useMemo(
    () => buildCostLedger(turnUsages, pricingByModel, turnTimestamps),
    [turnTimestamps, turnUsages, pricingByModel]
  );

  // Three-lane session flow model for the usage panel's diagram.
  const diagram = useMemo(
    () =>
      buildWriterSessionDiagram(
        workspace?.turns ?? [],
        workspace?.resolutions ?? []
      ),
    [workspace?.turns, workspace?.resolutions]
  );

  const addReference = useCallback(
    (reference: WritingBlockReference) => {
      setQueuedReferences((current) => {
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
      setActivePane("write");
      requestAnimationFrame(() => composerRef.current?.focus());
    },
    [setSaveStatus]
  );

  const openRevisions = useCallback(() => setRevisionsOpen(true), []);

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

  async function handleSend() {
    const value = instruction.trim();
    if (!workspace || !value || composerDisabled) return;
    const sent = await send(
      selectedModelId,
      value,
      references.map((reference) => ({
        section_id: reference.sectionId,
        block_index: reference.blockIndex,
      }))
    );
    if (sent) {
      setInstruction("");
      setQueuedReferences([]);
    }
  }

  async function handleGenerateFullDraft(replacementConfirmed = false) {
    if (!workspace || composerDisabled || !selectedModelId) return;
    const hasExistingDraft =
      workspace.draft.content.sections.length > 0 ||
      workspace.draft.content.title !== "Untitled report";
    if (hasExistingDraft && !replacementConfirmed) {
      setFullDraftConfirmationOpen(true);
      return;
    }
    setFullDraftConfirmationOpen(false);
    const generated = await generateFullDraft(
      selectedModelId,
      instruction.trim()
    );
    if (generated) {
      setInstruction("");
      setQueuedReferences([]);
      setActivePane("write");
    }
  }

  async function handleApplyTemplate() {
    if (editing) return;
    const applied = await applyTemplate(selectedTemplateId);
    if (applied) void reloadTemplates();
  }

  const contextPills = buildContextPills(workspace, references, liveContext);

  return (
    <>
      <div className="flex-1 min-h-0 grid grid-cols-1 min-[800px]:grid-cols-[minmax(340px,42%)_minmax(0,58%)] overflow-y-auto min-[800px]:overflow-hidden">
      <section className="min-h-[32rem] min-[800px]:min-h-0 flex flex-col bg-white">
        <div className="space-y-2 px-5 py-3">
          <div className="grid grid-cols-[minmax(4rem,1fr)_minmax(6rem,0.9fr)_auto] items-center gap-2">
            <div className="min-w-0">
              <EditableName
                value={workspace.session_name}
                label="writer session"
                onSave={renameSession}
                disabled={controlsBusy}
                compactActions
                className="w-full text-sm"
              />
            </div>
            <ModelSelect
              models={chatModels}
              loading={chatModelsLoading}
              error={chatModelsError}
              value={selectedModelId}
              onChange={setSelectedModelId}
              disabled={controlsBusy || pending !== null}
              ariaLabel="Writing model"
              className="min-w-0 w-full"
            />
            <button
              type="button"
              aria-expanded={contextOpen}
              aria-controls="writing-context-control"
              onClick={() => setContextOpen((open) => !open)}
              className="rounded-md border border-gray-300 bg-white px-3 py-1.5 text-xs font-medium hover:bg-gray-50"
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

        <SessionTabs
          idPrefix="writer-session"
          label="Writing session"
          active={activePane}
          onSelect={setActivePane}
          tabs={[
            { id: "setup", label: "Get started" },
            { id: "write", label: "Write with Claude" },
            {
              id: "usage",
              label: "Costs and cache",
              compact: true,
              icon: <UsageTabIcon />,
            },
          ]}
        />

        {activePane === "setup" && (
          <div
            id="writer-session-panel-setup"
            role="tabpanel"
            aria-labelledby="writer-session-tab-setup"
            className="flex-1 overflow-y-auto bg-gray-50 px-5 py-5"
            data-testid="writer-setup"
          >
            <div className="mx-auto max-w-xl space-y-4">
              <div>
                <h3 className="text-sm font-semibold text-gray-900">
                  Start this report
                </h3>
                <p className="mt-1 text-xs leading-5 text-gray-500">
                  Choose a template, ask Claude to fill the whole report, or skip both and start with tools. Every step is optional.
                </p>
              </div>

              <section className="rounded-lg border border-gray-200 bg-white p-4">
                <div className="mb-3 flex items-start gap-3">
                  <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-gray-100 text-[10px] font-semibold text-gray-600">
                    1
                  </span>
                  <div>
                    <h4 className="text-xs font-semibold text-gray-800">
                      Choose a Word template <span className="font-normal text-gray-400">· optional</span>
                    </h4>
                    <p className="mt-0.5 text-[11px] text-gray-500">
                      Start with saved headings, tables, and export formatting.
                    </p>
                  </div>
                </div>

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
                    <label className="block min-w-0">
                      <span className="sr-only">Writer template</span>
                      <select
                        aria-label="Writer template"
                        value={selectedTemplateId}
                        onChange={(event) => setChosenTemplateId(event.target.value)}
                        disabled={controlsBusy || pending !== null || writerTemplates.length === 0}
                        className="w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50"
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
                    <div className="mt-2 flex flex-wrap items-center justify-between gap-2">
                      <button
                        type="button"
                        onClick={onManageTemplates}
                        disabled={controlsBusy}
                        className="text-xs font-medium text-blue-700 hover:text-blue-900 disabled:opacity-50"
                      >
                        Manage templates
                      </button>
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
                        className="rounded-md border border-gray-300 bg-white px-3 py-1.5 text-xs font-medium hover:bg-gray-50 disabled:opacity-50"
                      >
                        {busy === "applying_template" ? "Applying…" : "Apply template"}
                      </button>
                    </div>
                    {templatesError && (
                      <p role="alert" className="mt-2 text-xs text-red-600">
                        Could not load writer templates: {templatesError}
                      </p>
                    )}
                  </>
                )}
              </section>

              {workspace.turns.length === 0 && (
                <section className="rounded-lg border border-blue-200 bg-white p-4">
                  <div className="mb-3 flex items-start gap-3">
                    <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-blue-100 text-[10px] font-semibold text-blue-700">
                      2
                    </span>
                    <div>
                      <h4 className="text-xs font-semibold text-gray-800">
                        Fill the whole report <span className="font-normal text-gray-400">· optional</span>
                      </h4>
                      <p className="mt-0.5 text-[11px] leading-4 text-gray-500">
                        Claude reads every available record and saves one complete, versioned draft.
                      </p>
                    </div>
                  </div>
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-[11px] font-medium text-gray-600">
                      Guidance <span className="font-normal text-gray-400">· optional</span>
                    </span>
                    <WriterPromptPicker
                      prompts={writerPrompts}
                      currentValue={instruction}
                      disabled={composerDisabled}
                      onPick={insertSavedPrompt}
                      onManage={onManagePrompts}
                    />
                  </div>
                  <label className="block">
                    <span className="sr-only">Full report guidance</span>
                    <textarea
                      ref={composerRef}
                      aria-label="Full report guidance"
                      value={instruction}
                      onChange={(event) => setInstruction(event.currentTarget.value)}
                      disabled={composerDisabled}
                      rows={3}
                      placeholder="For example: Use a concise clinical style…"
                      className="mt-1 w-full resize-y rounded-md border border-gray-300 bg-white px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50"
                    />
                  </label>
                  <div className="mt-3 flex justify-end">
                    <button
                      type="button"
                      onClick={() => void handleGenerateFullDraft()}
                      disabled={composerDisabled}
                      className="rounded-md bg-blue-700 px-3 py-2 text-xs font-semibold text-white hover:bg-blue-800 disabled:opacity-50"
                    >
                      {busy === "generating" ? "Filling…" : "Fill whole report"}
                    </button>
                  </div>
                  {actionError && (
                    <div
                      role="alert"
                      className="mt-3 rounded-md border border-red-200 bg-red-50 p-3 text-xs text-red-700"
                    >
                      <p className="font-semibold">Could not complete the Writer action</p>
                      <p className="mt-1 break-words">{actionError}</p>
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
                </section>
              )}

              <div className="flex flex-wrap items-center justify-between gap-3 border-t border-gray-200 pt-4">
                <p className="text-[11px] text-gray-500">
                  Prefer to build it section by section with approved tools?
                </p>
                <button
                  type="button"
                  onClick={() => setActivePane("write")}
                  className="text-xs font-semibold text-blue-700 hover:text-blue-900"
                >
                  Write with Claude →
                </button>
              </div>

              {saveStatus && (
                <p role="status" aria-live="polite" className="text-xs text-gray-600">
                  {saveStatus}
                </p>
              )}
            </div>
          </div>
        )}

        {activePane === "usage" && (
          <div
            id="writer-session-panel-usage"
            role="tabpanel"
            aria-labelledby="writer-session-tab-usage"
            className="min-h-0 flex-1"
          >
            <SessionUsagePanel
              session={session}
              ledger={ledger}
              showTurnCosts={showTurnCosts}
              onShowTurnCostsChange={setShowTurnCosts}
              turnCostsLabel="Writing timeline"
              diagram={diagram}
            />
          </div>
        )}

        {activePane === "write" && (
          <div
            id="writer-session-panel-write"
            role="tabpanel"
            aria-labelledby="writer-session-tab-write"
            className="flex min-h-0 flex-1 flex-col"
          >
        <div
          aria-label="Writing timeline"
          className="flex-1 overflow-y-auto px-5 py-4 space-y-4 select-text"
        >
          {workspace.turns.length === 0 && !pending && (
            <ChatEmptyState
              title="Build the report interactively."
              subtitle="Ask Claude to inspect records with approved tools, answer a question, or propose specific sections."
            />
          )}
          {workspace.turns.map((turn) => (
            <div key={turn.id} className="space-y-2">
              {turn.timeline.map((item, index) => (
                <TimelineItem key={`${turn.id}-${index}`} item={item} />
              ))}
              <div className="flex items-center justify-end gap-2 text-[10px] text-gray-400">
                <span>
                  {turn.tool_uses} tool use{turn.tool_uses === 1 ? "" : "s"}
                </span>
                {showTurnCosts && <TurnCostBadge usage={turn.usage} />}
              </div>
            </div>
          ))}

          {pending && (
            <div ref={proposalStartRef}>
              <WritingProposalCard
                proposal={pending}
                accepted={workspace.draft.content}
                busy={busy === "resolving"}
                onAccept={() => void resolveProposal("accept")}
                onReject={() => void resolveProposal("reject")}
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
            <div
              className="mb-2 flex items-center gap-2 text-xs text-amber-700"
              data-testid="queued-report-edits"
            >
              <p className="min-w-0 flex-1">
                {dirty
                  ? `${editCount} report edit${editCount === 1 ? "" : "s"} queued. Claria will save and include them with your next message.`
                  : `Accepted report r${workspace.draft.revision} has saved changes since Claude saw r${workspace.last_agent_revision ?? 0}; they are queued for your next message.`}
              </p>
              <button
                type="button"
                onClick={() => void discardQueuedEdits()}
                disabled={controlsBusy}
                className="shrink-0 font-semibold text-amber-800 hover:text-amber-950 disabled:opacity-50"
              >
                {busy === "discarding" ? "Discarding…" : "Discard"}
              </button>
            </div>
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
                      setQueuedReferences((current) =>
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
          <div className="mb-2 flex justify-end">
            <WriterPromptPicker
              prompts={writerPrompts}
              currentValue={instruction}
              disabled={composerDisabled}
              onPick={insertSavedPrompt}
              onManage={onManagePrompts}
            />
          </div>
          <ChatComposer
            composerRef={composerRef}
            ariaLabel="Writing instruction"
            value={instruction}
            onChange={setInstruction}
            onSend={() => void handleSend()}
            disabled={composerDisabled}
            canSend={!composerDisabled && instruction.trim() !== ""}
            placeholder="Ask a question or describe the report change you want…"
            sendLabel={busy === "sending" ? "Using tools…" : "Send"}
            onStop={stopRun}
            canStop={canStopRun}
            stopLabel="Stop"
            rows={4}
          />
        </div>
          </div>
        )}
      </section>

      <WritingCanvas
        workspace={workspace}
        edit={edit}
        editing={editing}
        dirty={dirty}
        busy={controlsBusy}
        onBeginEdit={beginEdit}
        onCancelEdit={cancelEdit}
        hasQueuedEdits={editsQueued}
        onDiscardQueued={() => void discardQueuedEdits()}
        onChange={setEdit}
        onSave={save}
        onExport={exportDocx}
        onOpenRevisions={openRevisions}
        onReference={addReference}
        status={exportStatus}
        validationErrors={validationErrors}
        agentActivity={agentActivity}
        run={run}
        runError={actionError}
        canStopRun={canStopRun}
        onStopRun={stopRun}
        onResumeRun={resumeRun}
        onKeepPartialDraft={keepPartialDraft}
        onDiscardRun={discardRun}
      />
      </div>

      {fullDraftConfirmationOpen && (
        <Modal
          open
          title="Replace the working draft?"
          onClose={() => setFullDraftConfirmationOpen(false)}
          className="max-w-lg p-6"
        >
          <p className="text-sm leading-6 text-gray-600">
            Claude will fill the whole report from every readable client record
            and replace revision {workspace.draft.revision} with one new saved
            revision. The current revision will remain available under
            Revisions.
          </p>
          <div className="mt-5 flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setFullDraftConfirmationOpen(false)}
              className="rounded-md border border-gray-300 bg-white px-3 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => void handleGenerateFullDraft(true)}
              className="rounded-md bg-blue-700 px-3 py-2 text-sm font-semibold text-white hover:bg-blue-800"
            >
              Fill whole report
            </button>
          </div>
        </Modal>
      )}
      {previewFilename && (
        <RecordFilePreviewModal
          clientId={clientId}
          filename={previewFilename}
          readOnly
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
            applyReverted(updated);
            setQueuedReferences([]);
          }}
        />
      )}
    </>
  );
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
