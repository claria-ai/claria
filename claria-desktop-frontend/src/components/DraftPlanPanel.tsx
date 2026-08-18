import { useCallback, useMemo, useState } from "react";
import type { DraftRun, PlanEntryEdit } from "../lib/tauri";
import {
  FRESH_INTENTS,
  RESUME_INTENTS,
  INTENT_LABELS,
  changedPlanEdits,
  emptyRestrictionCount,
  plannedSectionCount,
  planRows,
  remainingSectionCount,
  type DraftPlanMode,
  type PlanRow,
} from "../lib/draftPlan";
import type { DraftRunUiState } from "../lib/draftRun";
import { useRecordFiles } from "../lib/useRecordFiles";
import { logFrontendEvent } from "../lib/logBridge";
import DraftPlanCard from "./DraftPlanCard";
import Modal from "./Modal";
import ProgressBar from "./ProgressBar";
import Spinner from "./Spinner";
import type { StatusChipTone } from "./StatusChip";

/** Live section state, as the trailing chip on a card in a running plan. */
const LIVE_CHIP: Record<
  string,
  { tone: StatusChipTone; label: string; animated?: boolean }
> = {
  pending: { tone: "neutral", label: "Waiting" },
  drafting: { tone: "progress", label: "Writing", animated: true },
  drafted: { tone: "success", label: "Drafted" },
  failed: { tone: "danger", label: "Failed" },
  skipped: { tone: "muted", label: "Skipped" },
  kept: { tone: "neutral", label: "Unchanged" },
};

const DIRECTIVE_TONE: Record<string, StatusChipTone> = {
  draft: "info",
  rewrite: "progress",
  keep: "neutral",
  skip: "muted",
};

/**
 * The Draft run pane: the section plan a whole-report draft was made from,
 * editable before it runs and readable while it does.
 *
 * The pane owns the reader's unsaved plan edits, so callers must mount it
 * under a key that changes with the run and the mode — the edit buffer is
 * never reconciled against incoming props.
 */
export default function DraftPlanPanel({
  clientId,
  run,
  runState,
  mode,
  busy,
  error,
  canStop,
  onStop,
  onStart,
  onCancelPlan,
}: {
  clientId: string;
  /** The durable run behind the plan. `null` while the plan pass is running. */
  run: DraftRun | null;
  runState: DraftRunUiState;
  mode: DraftPlanMode;
  busy: boolean;
  error: string | null;
  canStop: boolean;
  onStop: () => void;
  /** Approve the plan: the edits to flush first, then the run-wide guidance. */
  onStart: (edits: PlanEntryEdit[], instructions: string) => void;
  onCancelPlan: () => void;
}) {
  const kind = mode === "resume-gate" ? "resume" : "fresh";
  const original = useMemo(
    () => (run ? planRows(run, kind) : []),
    [kind, run]
  );
  const [rows, setRows] = useState<PlanRow[]>(original);
  const [instructions, setInstructions] = useState("");
  const [confirmingCancel, setConfirmingCancel] = useState(false);
  const [recordError, setRecordError] = useState<string | null>(null);

  const onRecordError = useCallback((message: string | null) => {
    setRecordError(message);
    if (message) {
      logFrontendEvent("error", `Draft plan record listing failed: ${message}`);
    }
  }, []);
  // Only mounted while a run exists, so the client's record list is fetched
  // exactly when the reader can attach one to a section.
  const { files: recordFiles, loading: recordsLoading } = useRecordFiles(
    clientId,
    onRecordError
  );
  const recordFilenames = useMemo(
    () => recordFiles.map((file) => file.filename),
    [recordFiles]
  );

  const updateRow = useCallback((next: PlanRow) => {
    setRows((current) =>
      current.map((row) => (row.sectionId === next.sectionId ? next : row))
    );
  }, []);

  const gated = mode !== "running";
  const intents = !gated
    ? []
    : mode === "resume-gate"
      ? RESUME_INTENTS
      : FRESH_INTENTS;
  const warnings = run?.plan?.plan_warnings ?? [];
  const plannedGuidance = run?.instructions[0]?.text ?? "";
  const total = runState.total ?? rows.length;
  // A restriction switched on but left empty would be a section drafted from
  // nothing. The backend refuses it; the gate says so before the reader gets
  // that far.
  const unfilledRestrictions = emptyRestrictionCount(rows);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="space-y-2 border-b border-gray-200 bg-white px-5 py-3">
        <div className="flex items-center gap-3">
          <p className="min-w-0 flex-1 text-sm font-semibold text-gray-900">
            {phaseLine(mode, runState, rows.length)}
          </p>
          {mode === "running" && (
            <button
              type="button"
              onClick={onStop}
              disabled={!canStop}
              className="shrink-0 rounded-md px-2.5 py-1 text-xs font-medium text-gray-600 hover:bg-gray-100 hover:text-gray-900 disabled:opacity-50"
            >
              {runState.stopping ? "Stopping…" : "Stop run"}
            </button>
          )}
        </div>
        {/* The same numbers the canvas strip draws: one run, one denominator. */}
        {runState.total !== null ? (
          <ProgressBar
            label="Report sections drafted"
            value={runState.drafted}
            max={runState.total}
            valueText={`${runState.drafted} of ${runState.total} drafted`}
            showValueText={false}
          />
        ) : (
          runState.planTotal !== null && (
            <ProgressBar
              label="Report sections planned"
              value={runState.planned}
              max={runState.planTotal}
              valueText={`${runState.planned} of ${runState.planTotal} planned`}
              showValueText={false}
            />
          )
        )}
      </div>

      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto bg-gray-50 px-5 py-4">
        {run === null ? (
          <div className="flex items-center gap-2 py-6 text-sm text-gray-500">
            <Spinner />
            <span>Reading the records and deciding what each section needs…</span>
          </div>
        ) : (
          <>
            {warnings.length > 0 && (
              <div
                data-testid="plan-warnings"
                className="rounded-md border border-amber-200 bg-amber-50 p-3"
              >
                <p className="text-xs font-semibold text-amber-900">
                  Some evidence could not be matched to this client's records.
                  Fix or remove it before drafting.
                </p>
                <ul className="mt-1 list-disc pl-5 text-xs text-amber-800">
                  {warnings.map((warning) => (
                    <li key={warning}>{warning}</li>
                  ))}
                </ul>
              </div>
            )}

            {mode === "resume-gate" ? (
              <label className="block">
                <span className="text-xs font-medium text-gray-700">
                  Updated instructions for this run{" "}
                  <span className="font-normal text-gray-400">· optional</span>
                </span>
                <textarea
                  aria-label="Updated instructions for this run"
                  value={instructions}
                  disabled={busy}
                  rows={2}
                  onChange={(event) =>
                    setInstructions(event.currentTarget.value)
                  }
                  placeholder="What should change when it starts back up?"
                  className="mt-1 w-full resize-y rounded-md border border-gray-300 bg-white px-2.5 py-1.5 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-50"
                />
                <span className="mt-1 block text-[11px] text-gray-500">
                  Instructions re-plan the run before it picks back up. Leave
                  this empty to carry on with the sections below.
                </span>
              </label>
            ) : (
              plannedGuidance !== "" && (
                <p className="text-xs text-gray-500">
                  Planned against your guidance:{" "}
                  <span className="text-gray-700">{plannedGuidance}</span>
                </p>
              )
            )}

            {recordError && (
              <p role="alert" className="text-xs text-red-600">
                Could not list this client's records, so evidence cannot be
                added: {recordError}
              </p>
            )}

            <div className="space-y-2">
              {rows.map((row) => (
                <DraftPlanCard
                  key={row.sectionId}
                  row={row}
                  intents={intents}
                  chip={chipFor(mode, row, runState)}
                  disabled={busy}
                  recordFilenames={recordsLoading ? [] : recordFilenames}
                  onChange={updateRow}
                />
              ))}
            </div>
          </>
        )}

        {error && (
          <p
            role="alert"
            className="rounded-md border border-red-200 bg-red-50 p-3 text-xs text-red-700"
          >
            {error}
          </p>
        )}
      </div>

      {gated && run !== null && (
        <div className="flex items-center gap-2 border-t border-gray-200 bg-white px-5 py-3">
          <button
            type="button"
            disabled={busy || unfilledRestrictions > 0}
            title={
              unfilledRestrictions > 0
                ? "A section is restricted to selected records but none are selected."
                : undefined
            }
            onClick={() =>
              onStart(changedPlanEdits(original, rows), instructions.trim())
            }
            className="rounded-md bg-blue-700 px-3 py-2 text-xs font-semibold text-white hover:bg-blue-800 disabled:opacity-50"
          >
            {mode === "resume-gate"
              ? `Start back up (${remainingSectionCount(rows)} remaining)`
              : `Start drafting (${plannedSectionCount(rows)} sections)`}
          </button>
          {mode === "plan-gate" && (
            <button
              type="button"
              disabled={busy}
              onClick={() => setConfirmingCancel(true)}
              className="rounded-md border border-gray-300 bg-white px-3 py-2 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
            >
              Cancel plan
            </button>
          )}
          <span className="ml-auto text-[11px] text-gray-500">
            {total} section{total === 1 ? "" : "s"} in this report
          </span>
        </div>
      )}

      {confirmingCancel && (
        <Modal
          open
          title="Cancel this plan?"
          onClose={() => setConfirmingCancel(false)}
          className="max-w-lg p-6"
        >
          <p className="text-sm leading-6 text-gray-600">
            The plan is thrown away and nothing is written. The accepted report
            is left exactly as it is, and you can plan it again at any time.
          </p>
          <div className="mt-5 flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setConfirmingCancel(false)}
              className="rounded-md border border-gray-300 bg-white px-3 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50"
            >
              Keep the plan
            </button>
            <button
              type="button"
              onClick={() => {
                setConfirmingCancel(false);
                onCancelPlan();
              }}
              className="rounded-md bg-red-700 px-3 py-2 text-sm font-semibold text-white hover:bg-red-800"
            >
              Cancel plan
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}

/** What the run is doing, in one line, with no invented percentages. */
function phaseLine(
  mode: DraftPlanMode,
  runState: DraftRunUiState,
  sectionCount: number
): string {
  if (mode === "plan-gate") return "Plan ready — review before drafting";
  if (mode === "resume-gate") {
    const total = runState.total ?? sectionCount;
    return runState.outcome === "failed"
      ? `Stopped by an error — ${runState.drafted} of ${total} sections drafted and saved`
      : `Stopped — ${runState.drafted} of ${total} sections drafted and saved`;
  }
  if (runState.total === null) {
    // A retry outranks the row count: the count has stopped moving, and
    // saying why is more use than repeating the number it stopped on.
    if (runState.retrying) {
      return `Reconnecting to Claude — attempt ${runState.retrying.attempt} of ${runState.retrying.maxAttempts}`;
    }
    return runState.planTotal === null
      ? "Planning the report…"
      : `Planning — ${runState.planned} of ${runState.planTotal} sections decided`;
  }
  return `Drafting — ${runState.drafted} of ${runState.total}`;
}

function chipFor(
  mode: DraftPlanMode,
  row: PlanRow,
  runState: DraftRunUiState
): { tone: StatusChipTone; label: string; animated?: boolean } {
  if (mode === "running") {
    const live = runState.sections.get(row.sectionId);
    if (live) return LIVE_CHIP[live.status];
    return { tone: "neutral", label: "Waiting" };
  }
  return { tone: DIRECTIVE_TONE[row.intent], label: INTENT_LABELS[row.intent] };
}
