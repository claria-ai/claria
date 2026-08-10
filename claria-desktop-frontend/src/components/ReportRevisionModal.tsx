import { useEffect, useState } from "react";
import {
  listReportRevisions,
  loadReportRevision,
  revertReportRevision,
  type ReportDraftView,
  type ReportRevisionView,
  type ReportWorkspaceView,
} from "../lib/tauri";
import { formatDateTime } from "../lib/format";
import Modal from "./Modal";
import Spinner from "./Spinner";
import { ReportDocument } from "./WritingCanvas";

/** Browse immutable report revisions and restore one as a new revision. */
export default function ReportRevisionModal({
  clientId,
  workspace,
  canRevert,
  onClose,
  onReverted,
}: {
  clientId: string;
  workspace: ReportWorkspaceView;
  canRevert: boolean;
  onClose: () => void;
  onReverted: (workspace: ReportWorkspaceView) => void;
}) {
  const [revisions, setRevisions] = useState<ReportRevisionView[]>([]);
  const [selectedRevision, setSelectedRevision] = useState<number | null>(null);
  const [draft, setDraft] = useState<ReportDraftView | null>(null);
  const [loadingList, setLoadingList] = useState(true);
  const [loadingDraft, setLoadingDraft] = useState(false);
  const [reverting, setReverting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    setLoadingList(true);
    setError(null);
    void listReportRevisions(clientId, workspace.report_id).then(
      (values) => {
        if (!current) return;
        const previous = values.filter(
          (value) => value.revision < workspace.draft.revision
        );
        setRevisions(previous);
        setSelectedRevision(previous[0]?.revision ?? null);
        setLoadingList(false);
      },
      (reason) => {
        if (!current) return;
        setError(String(reason));
        setLoadingList(false);
      }
    );
    return () => {
      current = false;
    };
  }, [clientId, workspace.draft.revision, workspace.report_id]);

  useEffect(() => {
    if (selectedRevision === null) return;
    let current = true;
    setLoadingDraft(true);
    setError(null);
    void loadReportRevision(
      clientId,
      workspace.report_id,
      selectedRevision
    ).then(
      (value) => {
        if (!current) return;
        setDraft(value);
        setLoadingDraft(false);
      },
      (reason) => {
        if (!current) return;
        setError(String(reason));
        setLoadingDraft(false);
      }
    );
    return () => {
      current = false;
    };
  }, [clientId, selectedRevision, workspace.report_id]);

  async function revert() {
    if (selectedRevision === null || reverting || !canRevert) return;
    setReverting(true);
    setError(null);
    try {
      const updated = await revertReportRevision(
        clientId,
        workspace.report_id,
        workspace.draft.revision,
        selectedRevision
      );
      onReverted(updated);
      onClose();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setReverting(false);
    }
  }

  const selected = revisions.find(
    (revision) => revision.revision === selectedRevision
  );
  const selectedDraft =
    draft?.revision === selectedRevision ? draft : null;

  return (
    <Modal
      open
      onClose={onClose}
      title="Report revisions"
      variant="framed"
      dismissible={!reverting}
      className="max-w-6xl h-[90vh] flex flex-col overflow-hidden"
    >
      <div className="border-b border-gray-200 bg-gray-50 px-5 py-3 flex flex-wrap items-end gap-3">
        <label className="min-w-0 flex-1">
          <span className="text-xs font-medium text-gray-600">Previous revision</span>
          <select
            aria-label="Previous report revision"
            value={selectedRevision ?? ""}
            onChange={(event) => setSelectedRevision(Number(event.target.value))}
            disabled={loadingList || revisions.length === 0 || reverting}
            className="mt-1 w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm disabled:bg-gray-100"
          >
            {revisions.length === 0 && <option value="">No previous revisions</option>}
            {revisions.map((revision) => (
              <option key={revision.revision} value={revision.revision}>
                Revision {revision.revision} · {revision.title} · {formatDateTime(revision.updated_at)}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          onClick={() => void revert()}
          disabled={
            !canRevert || selectedDraft === null || loadingDraft || reverting
          }
          title={
            canRevert
              ? "Restore this content as a new report revision"
              : "Save or discard current work before restoring a revision"
          }
          className="mb-px rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50"
        >
          {reverting ? "Restoring…" : "Revert to version"}
        </button>
      </div>

      {error && (
        <p role="alert" className="mx-5 mt-3 text-sm text-red-600">
          {error}
        </p>
      )}
      <p className="px-5 pt-3 text-xs text-gray-500">
        {selected
          ? `Viewing revision ${selected.revision}. Restoring it creates revision ${workspace.draft.revision + 1}; no history is removed.`
          : "Every saved report revision remains available."}
      </p>

      <div className="min-h-0 flex-1 overflow-y-auto bg-gray-100 p-6">
        {loadingList || loadingDraft ? (
          <div role="status" className="flex items-center justify-center gap-2 py-16 text-sm text-gray-500">
            <Spinner />
            Loading report revision…
          </div>
        ) : selectedDraft ? (
          <div className="mx-auto min-h-full max-w-3xl rounded-sm border border-gray-200 bg-white px-10 py-12 shadow-sm select-text">
            <ReportDocument
              content={selectedDraft.content}
              testId="revision-report-canvas"
            />
          </div>
        ) : (
          <div className="py-16 text-center text-sm text-gray-500">
            This report does not have an earlier saved revision yet.
          </div>
        )}
      </div>
    </Modal>
  );
}
