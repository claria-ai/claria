import { Comparison, PlainText } from "./WritingDiff";
import StatusChip from "./StatusChip";
import type { Finding } from "../lib/tauri";
import {
  reviewPropertyLabel,
  type FindingState,
} from "../lib/findings";

/**
 * Where a card points the composer. The card knows the anchor, not the
 * document, so the page resolves the block itself against the workspace it
 * already holds.
 */
export type FindingReference = {
  sectionId: string;
  /** `null` when the finding never resolved to a block. */
  blockIndex: number | null;
};

/**
 * One review finding, in the shape its pass earns.
 *
 * A style finding carries an anchored replacement and gets Apply; a
 * consistency finding never does. That is not a rendering choice — the
 * consistency pass has no write access and the validator refuses a proposal
 * on one — so this component offers no fix path for it at all.
 */
export default function FindingCard({
  finding,
  state,
  quote,
  conflictingHeading,
  currentRevision,
  busy,
  resolving,
  onApply,
  onUndo,
  onDismiss,
  onReference,
  onPreviewRecord,
}: {
  finding: Finding;
  state: FindingState;
  /** The anchored passage, sliced from the section. */
  quote: string | null;
  /** Heading of the section a consistency finding conflicts with. */
  conflictingHeading: string | null;
  /** The anchored section's revision now, for the invalidation note. */
  currentRevision: number;
  /** Any writer action is in flight. */
  busy: boolean;
  /** This card's own action is the one in flight. */
  resolving: boolean;
  onApply: (findingId: string) => void;
  onUndo: (findingId: string) => void;
  onDismiss: (findingId: string) => void;
  onReference: (reference: FindingReference) => void;
  onPreviewRecord: (filename: string) => void;
}) {
  const invalid = state === "invalidated";
  const resolved = state === "applied" || state === "dismissed";
  return (
    <div
      data-testid="finding-card"
      data-finding-id={finding.id}
      data-finding-state={state}
      className={
        invalid || resolved
          ? "rounded-md border border-gray-200 bg-white p-3"
          : "rounded-md border border-amber-200 bg-white p-3"
      }
    >
      <div className="flex items-start gap-2">
        <p
          className={
            invalid
              ? "min-w-0 flex-1 text-xs leading-5 text-gray-400 line-through"
              : resolved
                ? "min-w-0 flex-1 text-xs leading-5 text-gray-500"
                : "min-w-0 flex-1 text-xs leading-5 text-gray-800"
          }
        >
          {finding.description}
        </p>
        <StatusChip
          tone={invalid || resolved ? "muted" : "warning"}
          label={reviewPropertyLabel(finding.property)}
        />
      </div>

      {invalid && (
        <p className="mt-2 text-[11px] leading-4 text-gray-400">
          This section changed after the review (r{finding.anchor.revision} → r
          {currentRevision}). Re-run the review to refresh.
        </p>
      )}

      {state === "applied" && (
        <p
          data-testid="finding-receipt"
          className="mt-2 text-[11px] text-gray-600"
        >
          Applied in r{finding.applied_revision ?? "?"} ·{" "}
          <button
            type="button"
            disabled={busy}
            onClick={() => onUndo(finding.id)}
            className="font-semibold text-blue-700 hover:text-blue-900 disabled:opacity-50"
          >
            {resolving ? "Undoing…" : "Undo"}
          </button>
        </p>
      )}

      {state === "dismissed" && (
        <p
          data-testid="finding-receipt"
          className="mt-2 text-[11px] text-gray-500"
        >
          Dismissed
        </p>
      )}

      {!invalid && !resolved && (
        <>
          {finding.proposal ? (
            <div className="mt-2">
              <Comparison
                current={<PlainText text={finding.proposal.original_text} />}
                proposed={
                  <PlainText text={finding.proposal.replacement_text} />
                }
              />
            </div>
          ) : (
            <div className="mt-2 space-y-1.5">
              {quote && (
                <blockquote
                  data-testid="finding-quote"
                  className="border-l-2 border-gray-200 pl-2 text-[11px] leading-4 text-gray-600"
                >
                  {quote}
                </blockquote>
              )}
              {finding.conflicting && (
                <div data-testid="finding-conflict">
                  <p className="text-[11px] font-medium text-gray-500">
                    {conflictingHeading
                      ? `conflicts with ${conflictingHeading}`
                      : "conflicts with this section"}
                  </p>
                  <blockquote className="border-l-2 border-amber-200 pl-2 text-[11px] leading-4 text-gray-600">
                    {finding.conflicting.quote}
                  </blockquote>
                </div>
              )}
            </div>
          )}

          {finding.record_citation && (
            <button
              type="button"
              onClick={() =>
                onPreviewRecord(finding.record_citation?.filename ?? "")
              }
              className="mt-2 inline-flex max-w-full items-center gap-1 rounded-full border border-blue-200 bg-blue-50 px-2 py-0.5 text-[11px] text-blue-800 hover:bg-blue-100"
            >
              <span className="truncate">
                {finding.record_citation.filename}
              </span>
            </button>
          )}
        </>
      )}

      <div className="mt-2.5 flex flex-wrap items-center gap-2">
        {state === "open" && finding.proposal && (
          <button
            type="button"
            disabled={busy}
            onClick={() => onApply(finding.id)}
            className="rounded-md bg-blue-700 px-2.5 py-1 text-[11px] font-semibold text-white hover:bg-blue-800 disabled:opacity-50"
          >
            {resolving ? "Applying…" : "Apply"}
          </button>
        )}
        {!resolved && (
          <button
            type="button"
            disabled={busy}
            onClick={() => onDismiss(finding.id)}
            className="rounded-md border border-gray-300 bg-white px-2.5 py-1 text-[11px] font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
          >
            Dismiss
          </button>
        )}
        {state === "open" && !finding.proposal && (
          <button
            type="button"
            onClick={() =>
              onReference({
                sectionId: finding.anchor.section_id,
                blockIndex: finding.span?.block_index ?? null,
              })
            }
            className="text-[11px] font-semibold text-blue-700 hover:text-blue-900"
          >
            Reference in chat
          </button>
        )}
      </div>
    </div>
  );
}
