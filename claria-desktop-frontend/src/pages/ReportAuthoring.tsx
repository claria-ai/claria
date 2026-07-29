import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  exportReportDocx,
  loadReportWorkspace,
  resolveReportProposal,
  saveReportDraft,
  sendReportMessage,
  type ChatModel,
  type ReportDraftEdit,
  type ReportTimelineItemView,
  type ReportWorkspaceView,
} from "../lib/tauri";
import {
  draftToEdit,
  reportEditsEqual,
  validateReportEdit,
} from "../lib/reportWorkspace";
import ReportCanvas from "../components/ReportCanvas";
import ReportProposalCard from "../components/ReportProposalCard";
import Spinner from "../components/Spinner";

export type ReportLeaveState = {
  hasUnsavedWork: boolean;
  busy: boolean;
};

export default function ReportAuthoring({
  clientId,
  chatModels,
  chatModelsLoading,
  chatModelsError,
  preferredModelId,
  onLeaveStateChange,
  onRetryModels,
}: {
  clientId: string;
  chatModels: ChatModel[];
  chatModelsLoading: boolean;
  chatModelsError: string | null;
  preferredModelId?: string | null;
  onLeaveStateChange?: (state: ReportLeaveState) => void;
  onRetryModels?: () => void;
}) {
  const generationRef = useRef(0);
  const [workspace, setWorkspace] = useState<ReportWorkspaceView | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<
    null | "saving" | "sending" | "resolving" | "exporting"
  >(null);
  const [selectedModelId, setSelectedModelId] = useState("");
  const [instruction, setInstruction] = useState("");
  const [editing, setEditing] = useState(false);
  const [edit, setEdit] = useState<ReportDraftEdit>({
    title: "Untitled report",
    sections: [],
  });
  const [saveStatus, setSaveStatus] = useState<string | null>(null);
  const [exportStatus, setExportStatus] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
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
      setWorkspace(result);
      setEdit(draftToEdit(result.draft));
      setEditing(false);
    } catch (error) {
      if (generation !== generationRef.current) return;
      setLoadError(String(error));
    } finally {
      if (generation === generationRef.current) setLoading(false);
    }
  }, [clientId]);

  useEffect(() => {
    void load();
    return () => {
      generationRef.current += 1;
    };
  }, [load]);

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
  const validationErrors = useMemo(() => validateReportEdit(edit), [edit]);
  const hasUnsavedWork = dirty || instruction.trim() !== "";

  useEffect(() => {
    onLeaveStateChange?.({ hasUnsavedWork, busy: busy !== null });
    return () =>
      onLeaveStateChange?.({ hasUnsavedWork: false, busy: false });
  }, [busy, hasUnsavedWork, onLeaveStateChange]);

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
          <span>Loading report workspace…</span>
        </div>
      </div>
    );
  }

  if (loadError || !workspace) {
    return (
      <div className="flex-1 flex items-center justify-center bg-gray-50 p-8">
        <div className="max-w-md border border-red-200 bg-white rounded-lg p-6 text-center">
          <h3 className="text-sm font-semibold text-gray-900">
            Could not load the report workspace
          </h3>
          <p role="alert" className="text-sm text-red-600 mt-2">
            {loadError ?? "Unknown report workspace error"}
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
    dirty ||
    editing ||
    !selectedModelId ||
    chatModelsLoading;

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
        "Reloading will discard your local report edits. Your typed instruction will be kept. Continue?"
      )
    ) {
      return;
    }
    await load();
  }

  async function handleSave() {
    if (
      !workspace ||
      !dirty ||
      controlsBusy ||
      validationErrors.length > 0
    )
      return;
    const generation = generationRef.current;
    setBusy("saving");
    setActionError(null);
    setSaveStatus("Saving accepted draft…");
    try {
      const result = await saveReportDraft(
        clientId,
        workspace.draft.revision,
        edit
      );
      if (generation !== generationRef.current) return;
      setWorkspace(result);
      setEdit(draftToEdit(result.draft));
      setEditing(false);
      setConflict(false);
      setSaveStatus(`Saved revision ${result.draft.revision}.`);
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
    setSaveStatus("Claude is using the approved tools…");
    try {
      const result = await sendReportMessage(
        clientId,
        workspace.draft.revision,
        selectedModelId,
        value
      );
      if (generation !== generationRef.current) return;
      setWorkspace(result.workspace);
      setEdit(draftToEdit(result.workspace.draft));
      setInstruction("");
      setConflict(false);
      setSaveStatus(
        result.workspace.pending_proposal
          ? "Proposal ready for your review. The accepted draft is unchanged."
          : "Report assistant turn complete."
      );
    } catch (error) {
      if (generation !== generationRef.current) return;
      // Keep the instruction so the user can retry it verbatim.
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
    if (
      !window.confirm(
        "This report may contain PHI. Exporting creates an unencrypted local .docx outside Claria's managed storage. Continue?"
      )
    ) {
      setExportStatus("Export canceled.");
      return;
    }
    const generation = generationRef.current;
    const visibleReportId = workspace.report_id;
    const visibleRevision = workspace.draft.revision;
    setBusy("exporting");
    setActionError(null);
    setExportStatus("Preparing Word document…");
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
      setConflict(false);
      setExportStatus(
        result.exported
          ? `Word document exported from revision ${result.revision}.`
          : "Export canceled."
      );
    } catch (error) {
      if (generation !== generationRef.current) return;
      showActionError(error);
      setExportStatus(null);
    } finally {
      if (generation === generationRef.current) setBusy(null);
    }
  }

  return (
    <div className="flex-1 min-h-0 grid grid-cols-1 min-[800px]:grid-cols-[minmax(340px,42%)_minmax(0,58%)] overflow-y-auto min-[800px]:overflow-hidden">
      <section className="min-h-[32rem] min-[800px]:min-h-0 flex flex-col bg-white">
        <div className="px-5 py-4 border-b border-gray-200 space-y-3">
          <div className="rounded-md border border-blue-200 bg-blue-50 p-3">
            <p className="text-xs font-semibold text-blue-900">
              Tool-assisted report writing
            </p>
            <p className="text-xs leading-5 text-blue-800 mt-1">
              Claude can list and read bounded text from this client&apos;s record.
              It cannot change the report directly: every write is a proposal
              you must accept.
            </p>
          </div>
          <label className="block">
            <span className="text-xs font-medium text-gray-600">Model</span>
            <select
              aria-label="Report authoring model"
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
          aria-label="Report authoring timeline"
          className="flex-1 overflow-y-auto px-5 py-4 space-y-4"
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
              <ReportProposalCard
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
          <div role="alert" className="mx-5 mb-2 p-3 text-xs text-red-700 bg-red-50 border border-red-200 rounded-md">
            <p>{actionError}</p>
            {conflict && (
              <button
                type="button"
                onClick={() => void handleReload()}
                className="mt-2 font-semibold text-blue-700 hover:text-blue-900"
              >
                Reload workspace
              </button>
            )}
          </div>
        )}
        {saveStatus && (
          <p role="status" aria-live="polite" className="px-5 pb-2 text-xs text-gray-600">
            {saveStatus}
          </p>
        )}

        <div className="border-t border-gray-200 p-4">
          {(dirty || editing) && (
            <p className="text-xs text-amber-700 mb-2">
              Save or cancel manual edits before sending an instruction.
            </p>
          )}
          {pending && (
            <p className="text-xs text-violet-700 mb-2">
              Accept or reject the proposal before continuing.
            </p>
          )}
          <textarea
            aria-label="Report instruction"
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

      <ReportCanvas
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
        saveStatus={null}
        exportStatus={exportStatus}
        validationErrors={validationErrors}
      />
    </div>
  );
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
      <div className={`border rounded-md px-3 py-2 text-xs flex items-center gap-2 ${color}`}>
        <span aria-hidden="true">↳</span>
        <span className="flex-1">{item.summary}</span>
        <span className="capitalize">{item.status}</span>
      </div>
    );
  }

  const user = item.role === "user";
  return (
    <div className={`flex ${user ? "justify-end" : "justify-start"}`}>
      <div
        className={`max-w-[90%] rounded-lg px-3 py-2 text-sm whitespace-pre-wrap ${
          user
            ? "bg-blue-600 text-white"
            : "bg-gray-100 text-gray-800 border border-gray-200"
        }`}
      >
        {item.text}
      </div>
    </div>
  );
}
