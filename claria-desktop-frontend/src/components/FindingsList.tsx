import { useMemo, useState } from "react";
import FindingCard, { type FindingReference } from "./FindingCard";
import {
  anchoredQuote,
  conflictingHeading,
  currentSectionRevision,
  findingState,
  groupFindings,
  type FindingsFilter,
} from "../lib/findings";
import type { Finding, ReportContent } from "../lib/tauri";

const FILTERS: { id: FindingsFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "style", label: "Style" },
  { id: "consistency", label: "Consistency" },
  { id: "resolved", label: "Resolved" },
];

/**
 * Everything the review noticed, grouped by the section it points at and read
 * in document order.
 *
 * The list holds nothing but the chosen filter: findings, their resolution,
 * and what counts as stale all come from the workspace, so nothing here can
 * disagree with what the canvas is showing.
 */
export default function FindingsList({
  findings,
  content,
  draftRevision,
  busy,
  resolvingId,
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
  busy: boolean;
  /** The finding whose action is in flight, if any. */
  resolvingId: string | null;
  onApply: (findingId: string) => void;
  onUndo: (findingId: string) => void;
  onDismiss: (findingId: string) => void;
  onReference: (reference: FindingReference) => void;
  onPreviewRecord: (filename: string) => void;
  /** Lets the canvas flag chips scroll one section's cards into view. */
  registerSection?: (sectionId: string, element: HTMLElement | null) => void;
}) {
  const [filter, setFilter] = useState<FindingsFilter>("all");
  const groups = useMemo(
    () => groupFindings(findings, content, filter),
    [content, filter, findings]
  );

  return (
    <div data-testid="findings-list" className="space-y-3">
      <div
        role="group"
        aria-label="Filter findings"
        className="flex flex-wrap gap-1.5"
      >
        {FILTERS.map((option) => (
          <button
            key={option.id}
            type="button"
            aria-pressed={filter === option.id}
            onClick={() => setFilter(option.id)}
            className={
              filter === option.id
                ? "rounded-full border border-blue-300 bg-blue-50 px-2.5 py-0.5 text-[11px] font-medium text-blue-800"
                : "rounded-full border border-gray-200 bg-white px-2.5 py-0.5 text-[11px] font-medium text-gray-600 hover:bg-gray-50"
            }
          >
            {option.label}
          </button>
        ))}
      </div>

      {groups.length === 0 ? (
        <p className="text-xs text-gray-500">
          Nothing to show under this filter.
        </p>
      ) : (
        groups.map((group) => (
          <section
            key={group.sectionId === "" ? "removed" : group.sectionId}
            data-finding-section={group.sectionId}
            ref={(element) => registerSection?.(group.sectionId, element)}
            className="space-y-2"
          >
            <div className="flex items-baseline gap-2">
              <h4 className="min-w-0 flex-1 truncate text-xs font-semibold text-gray-900">
                {group.heading}
              </h4>
              <span className="shrink-0 text-[11px] text-gray-500 tabular-nums">
                {group.openCount} open
              </span>
            </div>
            {group.findings.map((finding) => (
              <FindingCard
                key={finding.id}
                finding={finding}
                state={findingState(finding, content)}
                quote={anchoredQuote(finding, content)}
                conflictingHeading={conflictingHeading(finding, content)}
                currentRevision={currentSectionRevision(
                  finding,
                  content,
                  draftRevision
                )}
                busy={busy}
                resolving={resolvingId === finding.id}
                onApply={onApply}
                onUndo={onUndo}
                onDismiss={onDismiss}
                onReference={onReference}
                onPreviewRecord={onPreviewRecord}
              />
            ))}
          </section>
        ))
      )}
    </div>
  );
}
