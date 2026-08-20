import { useState, type ReactNode } from "react";

import ProgressBar from "./ProgressBar";
import StatusChip from "./StatusChip";
import { formatDateTime } from "../lib/format";
import {
  countLabel,
  intentLabel,
  planOrigin,
  runOutcome,
  runSummaryLine,
  sectionOutcome,
  sectionReason,
  sectionRecords,
} from "../lib/draftRunHistory";
import type {
  DraftRunHistoryEntry,
  DraftRunHistorySection,
  DraftRunHistoryView,
} from "../lib/tauri";

/**
 * What was done to hydrate this report, kept after the run that did it.
 *
 * Three levels of disclosure, and the reason there are three is that the
 * questions are three: *did it finish* (the bar), *what was it told to do*
 * (the run's summary), and *why does this section say what it says* (the
 * section's row). A run that finished is one line and a full bar until
 * somebody asks otherwise.
 *
 * Everything here is read-only and comes from the run object in S3, so it
 * survives closing the report, reopening it, and restarting the app — which is
 * the whole point. The Start/Stop/Resume controls live in the plan panel
 * above; this pane never mutates anything.
 */
export default function DraftRunHistory({
  history,
  draftRevision,
  liveRunId,
  defaultRunId,
}: {
  history: DraftRunHistoryView;
  /** The revision on screen, so the run that wrote it can be named. */
  draftRevision: number;
  /**
   * The run this app is driving right now, if any. A stored run cannot say
   * whether it is in flight or was killed mid-pass, so the page that started
   * it is the only honest source.
   */
  liveRunId: string | null;
  /** Which run opens expanded, or `null` to open none. */
  defaultRunId: string | null;
}) {
  if (history.runs.length === 0) return null;
  return (
    <section
      data-testid="draft-run-history"
      aria-label="Drafting run history"
      className="space-y-2"
    >
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold text-gray-900">
          What was written, and from what
        </h3>
        <p className="text-[11px] text-gray-500">
          {countLabel(history.runs.length, "run")}, newest first
        </p>
      </div>
      {history.runs.map((run) => (
        <RunRow
          key={run.run_id}
          run={run}
          draftRevision={draftRevision}
          liveRunId={liveRunId}
          open={run.run_id === defaultRunId}
        />
      ))}
    </section>
  );
}

function RunRow({
  run,
  draftRevision,
  liveRunId,
  open,
}: {
  run: DraftRunHistoryEntry;
  draftRevision: number;
  liveRunId: string | null;
  open: boolean;
}) {
  // Seeded from `open`, then owned here. React writes `open` onto the DOM node
  // on every render, so a prop-driven `<details>` re-opens itself whenever the
  // page re-renders for an unrelated reason — a live run's progress events, for
  // one — and a row the reader closed would not stay closed.
  const [expanded, setExpanded] = useState(open);
  const outcome = runOutcome(run, liveRunId);
  const wroteThisRevision = run.finalized_revision === draftRevision;
  return (
    <details
      data-testid="draft-run-history-run"
      data-run-id={run.run_id}
      data-status={run.status}
      open={expanded}
      onToggle={(event) => setExpanded(event.currentTarget.open)}
      className="group rounded-lg border border-gray-200 bg-white"
    >
      <summary className="flex cursor-pointer list-none items-start gap-2 p-3 [&::-webkit-details-marker]:hidden">
        <span className="mt-0.5 shrink-0 text-xs text-gray-400 transition-transform group-open:rotate-90">
          &#9656;
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-center gap-2">
            <span className="truncate text-sm font-medium text-gray-800">
              {run.title ?? "Untitled draft"}
            </span>
            <StatusChip
              tone={outcome.tone}
              label={outcome.label}
              animated={outcome.live}
            />
            {wroteThisRevision && (
              <StatusChip tone="info" label="Wrote the report on screen" />
            )}
            {run.partial && (
              <StatusChip tone="warning" label="Kept from a stopped run" />
            )}
          </span>
          <span className="mt-1 block text-xs text-gray-500">
            {runSummaryLine(run)} · {formatDateTime(run.updated_at)}
          </span>
        </span>
        <ProgressBar
          className="mt-1 w-28 shrink-0"
          value={run.counts.decided}
          max={run.counts.total}
          label={`Sections decided in the run of ${formatDateTime(run.created_at)}`}
          valueText={`${run.counts.decided} of ${run.counts.total} sections`}
          showValueText={false}
        />
        <span className="mt-0.5 w-16 shrink-0 text-right text-[11px] tabular-nums text-gray-500">
          {run.counts.decided}/{run.counts.total}
        </span>
      </summary>

      <div className="space-y-3 border-t border-gray-100 px-3 py-3">
        <RunFacts run={run} />
        {run.instructions.length > 0 && (
          <div>
            <h4 className="text-[11px] font-semibold tracking-wide text-gray-500 uppercase">
              What the run was asked for
            </h4>
            <ul className="mt-1 space-y-1">
              {run.instructions.map((instruction, index) => (
                <li
                  key={`${instruction.added_at}-${index}`}
                  className="rounded border border-gray-100 bg-gray-50 px-2 py-1.5 text-xs whitespace-pre-wrap text-gray-700"
                >
                  {instruction.text}
                </li>
              ))}
            </ul>
          </div>
        )}
        {run.plan_warnings.length > 0 && (
          <div className="rounded border border-amber-200 bg-amber-50 px-2 py-1.5">
            <h4 className="text-[11px] font-semibold text-amber-800">
              What the host could not confirm about the plan
            </h4>
            <ul className="mt-1 list-disc space-y-0.5 pl-4 text-[11px] text-amber-800">
              {run.plan_warnings.map((warning) => (
                <li key={warning}>{warning}</li>
              ))}
            </ul>
          </div>
        )}
        <RecordSnapshot run={run} />
        <div>
          <h4 className="text-[11px] font-semibold tracking-wide text-gray-500 uppercase">
            Section by section
          </h4>
          <ul className="mt-1 space-y-1">
            {run.sections.map((section) => (
              <li key={section.section_id}>
                <SectionRow section={section} run={run} />
              </li>
            ))}
          </ul>
        </div>
      </div>
    </details>
  );
}

/** The run's own facts, as label/value pairs. */
function RunFacts({ run }: { run: DraftRunHistoryEntry }) {
  const facts: [string, string][] = [
    ["Plan", planOrigin(run)],
    ["Planning model", run.planner_model_id ?? "—"],
    ["Writing model", run.writer_model_id],
    [
      "Revision",
      run.finalized_revision === null
        ? `Built on revision ${run.base_revision}, cut none`
        : `${run.base_revision} → ${run.finalized_revision}`,
    ],
    ["Started", formatDateTime(run.created_at)],
  ];
  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
      {facts.map(([label, value]) => (
        <div key={label} className="contents">
          <dt className="text-gray-500">{label}</dt>
          <dd className="break-words text-gray-800">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

/**
 * The record corpus the run was built from — the answer to "what did it read".
 *
 * A run recorded before Claria captured one says so rather than showing an
 * empty list, because "no files" and "we did not write it down" are different
 * answers and only one of them is about the records.
 */
function RecordSnapshot({ run }: { run: DraftRunHistoryEntry }) {
  const [expanded, setExpanded] = useState(false);
  const snapshot = run.record_snapshot;
  if (snapshot === null) {
    return (
      <p className="text-[11px] text-gray-500">
        This run predates Claria recording which records it read.
      </p>
    );
  }
  const unreadable = snapshot.unavailable.length;
  return (
    <div>
      <h4 className="text-[11px] font-semibold tracking-wide text-gray-500 uppercase">
        Records in front of the writer
      </h4>
      <p className="mt-0.5 text-xs text-gray-700">
        {countLabel(snapshot.files.length, "record")} ·{" "}
        {snapshot.total_characters.toLocaleString()} characters
        {unreadable > 0 && ` · ${unreadable} could not be read`}
      </p>
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        className="mt-1 text-[11px] font-medium text-blue-700 hover:text-blue-900"
      >
        {expanded ? "Hide the file list" : "Show the file list"}
      </button>
      {expanded && (
        <ul className="mt-1 max-h-48 space-y-0.5 overflow-y-auto rounded border border-gray-100 bg-gray-50 p-2 text-[11px]">
          {snapshot.files.map((file) => (
            <li key={file.filename} className="flex justify-between gap-3">
              <span className="truncate text-gray-700">{file.filename}</span>
              <span className="shrink-0 tabular-nums text-gray-400">
                {file.characters.toLocaleString()}
              </span>
            </li>
          ))}
          {snapshot.unavailable.map((file) => (
            <li key={file.filename} className="flex justify-between gap-3">
              <span className="truncate text-gray-500 line-through">
                {file.filename}
              </span>
              <span className="shrink-0 text-amber-700">{file.reason}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function SectionRow({
  section,
  run,
}: {
  section: DraftRunHistorySection;
  run: DraftRunHistoryEntry;
}) {
  const outcome = sectionOutcome(section);
  const intent = intentLabel(section.intent);
  const reason = sectionReason(section);
  const records = sectionRecords(section.records, run.record_snapshot);
  return (
    <details
      data-testid="draft-run-history-section"
      data-section-id={section.section_id}
      data-state={section.state}
      className="group/section rounded border border-gray-200 bg-white"
    >
      <summary className="flex cursor-pointer list-none items-center gap-2 px-2.5 py-2 [&::-webkit-details-marker]:hidden">
        <span className="shrink-0 text-[10px] text-gray-400 transition-transform group-open/section:rotate-90">
          &#9656;
        </span>
        <span className="min-w-0 flex-1 truncate text-xs font-medium text-gray-800">
          {section.heading}
        </span>
        {section.required && <StatusChip tone="neutral" label="Required" />}
        <StatusChip tone={outcome.tone} label={outcome.label} />
      </summary>

      <div className="space-y-2 border-t border-gray-100 px-2.5 py-2 text-[11px]">
        {intent !== null && <Field label="The plan asked for">{intent}</Field>}
        {section.scope !== "" && <Field label="Scope">{section.scope}</Field>}
        {section.evidence.length > 0 && (
          <Field label="Evidence the planner named">
            <ul className="space-y-0.5">
              {section.evidence.map((ref) => (
                <li key={ref.filename}>
                  <span className="text-gray-800">{ref.filename}</span>
                  {ref.note !== null && (
                    <span className="text-gray-500"> — {ref.note}</span>
                  )}
                </li>
              ))}
            </ul>
          </Field>
        )}
        {section.instruction !== null && (
          <Field label="Directive for this section">
            {section.instruction}
          </Field>
        )}
        {records !== null && (
          <Field label="In the model call">
            <span>{records.label}</span>
            {records.filenames !== null && records.filenames.length > 0 && (
              <ul className="mt-0.5 space-y-0.5">
                {records.filenames.map((filename) => (
                  <li key={filename} className="text-gray-800">
                    {filename}
                  </li>
                ))}
              </ul>
            )}
          </Field>
        )}
        {section.citations.length > 0 && (
          <Field
            label={`Quoted from ${countLabel(
              new Set(section.citations.map((citation) => citation.filename))
                .size,
              "record",
            )}`}
          >
            <ul className="space-y-1">
              {section.citations.map((citation, index) => (
                <li key={`${citation.filename}-${index}`}>
                  <span className="text-gray-500">{citation.filename}</span>
                  <span className="block text-gray-800 italic">
                    “{citation.quote}”
                  </span>
                </li>
              ))}
            </ul>
          </Field>
        )}
        {section.state === "drafted" || section.state === "flagged" ? (
          <Field label="What it wrote">
            {countLabel(section.block_count, "block")} ·{" "}
            {section.characters.toLocaleString()} characters
            {section.attempts > 1 && ` · ${section.attempts} attempts`}
          </Field>
        ) : null}
        {reason !== null && <Field label="Why">{reason}</Field>}
      </div>
    </details>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <span className="block text-[10px] font-semibold tracking-wide text-gray-400 uppercase">
        {label}
      </span>
      <div className="mt-0.5 whitespace-pre-wrap text-gray-700">{children}</div>
    </div>
  );
}
