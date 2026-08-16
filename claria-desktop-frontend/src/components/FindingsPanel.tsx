import { useMemo } from "react";
import FindingsList from "./FindingsList";
import type { FindingReference } from "./FindingCard";
import ProgressBar from "./ProgressBar";
import { summarizeCompletion } from "../lib/findings";
import type { CompletionReport, Finding, ReportContent } from "../lib/tauri";

/**
 * The findings half of the Draft run pane: the button that asks for a review,
 * the review's own progress, what it found, and the completion checklist.
 *
 * It stands on its own — a report with findings and no drafting run shows
 * this and nothing else — so the plan panel above it can be absent.
 */
export default function FindingsPanel({
  findings,
  content,
  draftRevision,
  completion,
  busy,
  reviewing,
  reviewCompleted,
  reviewTotal,
  canReview,
  resolvingId,
  onReview,
  onApply,
  onUndo,
  onDismiss,
  onReference,
  onPreviewRecord,
  registerSection,
}: {
  findings: readonly Finding[];
  content: ReportContent;
  draftRevision: number;
  /** `null` before the first evaluation, and for a report with no revisions. */
  completion: CompletionReport | null;
  busy: boolean;
  reviewing: boolean;
  reviewCompleted: number;
  /** `null` until the first pass reports how many there are. */
  reviewTotal: number | null;
  canReview: boolean;
  resolvingId: string | null;
  onReview: () => void;
  onApply: (findingId: string) => void;
  onUndo: (findingId: string) => void;
  onDismiss: (findingId: string) => void;
  onReference: (reference: FindingReference) => void;
  onPreviewRecord: (filename: string) => void;
  registerSection?: (sectionId: string, element: HTMLElement | null) => void;
}) {
  const summary = useMemo(
    () => (completion ? summarizeCompletion(completion, content) : []),
    [completion, content]
  );

  return (
    <div
      data-testid="findings-panel"
      className="flex min-h-0 flex-1 flex-col border-t border-gray-200 bg-white"
    >
      <div className="flex items-center gap-3 px-5 py-2.5">
        <p className="min-w-0 flex-1 text-xs font-semibold text-gray-900">
          Review findings
        </p>
        <button
          type="button"
          onClick={onReview}
          disabled={!canReview}
          title={
            canReview
              ? undefined
              : "A saved revision with no pending proposal can be reviewed."
          }
          className="shrink-0 rounded-md border border-gray-300 bg-white px-2.5 py-1 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
        >
          {reviewing ? "Reviewing…" : "Review draft"}
        </button>
      </div>

      {reviewTotal !== null && (
        <div className="px-5 pb-2">
          <ProgressBar
            label="Review checks completed"
            value={reviewCompleted}
            max={reviewTotal}
            valueText={`${reviewCompleted} of ${reviewTotal} checks`}
          />
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto bg-gray-50 px-5 py-3">
        {findings.length === 0 ? (
          <p className="text-xs text-gray-500">
            No findings yet. Review the draft to check it for style and
            consistency.
          </p>
        ) : (
          <FindingsList
            findings={findings}
            content={content}
            draftRevision={draftRevision}
            busy={busy}
            resolvingId={resolvingId}
            onApply={onApply}
            onUndo={onUndo}
            onDismiss={onDismiss}
            onReference={onReference}
            onPreviewRecord={onPreviewRecord}
            registerSection={registerSection}
          />
        )}
      </div>

      {completion && (
        <div
          data-testid="completion-checklist"
          className="border-t border-gray-200 px-5 py-2.5"
        >
          {completion.complete ? (
            <p className="text-xs text-emerald-700">
              Ready — all checks pass
            </p>
          ) : (
            <ul className="space-y-0.5 text-xs text-gray-600">
              {summary.map((row) => (
                <li key={row.kind}>
                  {row.label}
                  {row.sections.length > 0 && (
                    <span className="text-gray-400">
                      {" "}
                      · {row.sections.join(", ")}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
