import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useChatModels } from "../lib/chatModels";
import ChatComposer from "../components/ChatComposer";
import ChatEmptyState from "../components/ChatEmptyState";
import ContextPills, { buildContextPills } from "../components/ContextPills";
import EditableName from "../components/EditableName";
import ModelSelect from "../components/ModelSelect";
import RecordFilePreviewModal from "../components/RecordFilePreviewModal";
import ReportRevisionModal from "../components/ReportRevisionModal";
import SessionTotalBanner from "../components/SessionTotalBanner";
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
import { usePreferredModel } from "../lib/usePreferredModel";
import { useReportWorkspace } from "../lib/useReportWorkspace";
import { useWriterTemplates } from "../lib/useWriterTemplates";
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
};

export default function Writing({
  clientId,
  expectedReportId,
  onLeaveStateChange,
  onManageTemplates,
}: {
  clientId: string;
  expectedReportId?: string | null;
  onLeaveStateChange?: (state: WritingLeaveState) => void;
  onManageTemplates: () => void;
}) {
  const {
    models: chatModels,
    loading: chatModelsLoading,
    error: chatModelsError,
    preferredModelId,
    retry: onRetryModels,
  } = useChatModels();
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  const initialComposerDraft = useRef(readWritingComposerDraft(clientId)).current;
  const [selectedModelId, setSelectedModelId] = usePreferredModel(
    chatModels,
    preferredModelId
  );
  const [instruction, setInstruction] = useState(
    initialComposerDraft?.instruction ?? ""
  );
  const [references, setReferences] = useState<WritingBlockReference[]>(
    initialComposerDraft?.references ?? []
  );
  const [contextOpen, setContextOpen] = useState(false);
  const [previewFilename, setPreviewFilename] = useState<string | null>(null);
  const [revisionsOpen, setRevisionsOpen] = useState(false);
  const [selectedTemplateId, setSelectedTemplateId] = useState("");
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
    load,
    beginEdit,
    cancelEdit,
    save,
    send,
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

  useEffect(() => {
    setSelectedTemplateId((current) =>
      writerTemplates.some((template) => template.id === current)
        ? current
        : (writerTemplates[0]?.id ?? "")
    );
  }, [writerTemplates]);

  useEffect(() => {
    writeWritingComposerDraft(clientId, { instruction, references });
  }, [clientId, instruction, references]);

  // Reconcile queued block references against the loaded draft: a reference
  // to a block that no longer exists (or changed kind) is dropped.
  useEffect(() => {
    if (!workspace) return;
    setReferences((current) => reconcileReferences(current, workspace));
    // A fresh workspace identity is the only trigger — references are
    // reconciled once per load/save result.
  }, [workspace]);

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

  // Lifetime writer-session spend, matching the chat surfaces' banner.
  const session: SessionUsage = useMemo(
    () =>
      (workspace?.turns ?? []).reduce(
        (acc, turn) => accumulateUsage(acc, turn.usage),
        EMPTY_SESSION_USAGE
      ),
    [workspace?.turns]
  );

  const addReference = useCallback(
    (reference: WritingBlockReference) => {
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
      setReferences([]);
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
        <div className="px-5 py-4 border-b border-gray-200 space-y-3">
          <div className="grid grid-cols-[minmax(4rem,0.7fr)_minmax(7rem,1fr)_auto] items-center gap-2">
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

        {/* Lifetime session spend, matching chat's banner. */}
        {session.turnCount > 0 && <SessionTotalBanner session={session} />}

        <div
          aria-label="Writing timeline"
          className="flex-1 overflow-y-auto px-5 py-4 space-y-4 select-text"
        >
          {workspace.turns.length === 0 && !pending && (
            <ChatEmptyState
              title="Build the report interactively."
              subtitle="Ask Claude to inspect records, answer a question, propose specific sections, or apply a managed Word template."
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
                <TurnCostBadge usage={turn.usage} />
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
            rows={4}
          />
        </div>
      </section>

      <WritingCanvas
        workspace={workspace}
        edit={edit}
        editing={editing}
        dirty={dirty}
        busy={controlsBusy}
        onBeginEdit={beginEdit}
        onCancelEdit={cancelEdit}
        onChange={setEdit}
        onSave={save}
        onExport={exportDocx}
        onOpenRevisions={openRevisions}
        onReference={addReference}
        status={exportStatus}
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
            applyReverted(updated);
            setReferences([]);
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
