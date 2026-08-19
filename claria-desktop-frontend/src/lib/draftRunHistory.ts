/**
 * Reading a drafting run after the fact.
 *
 * `draftRun.ts` is about a run that is happening: it folds progress events
 * into counters a bar can move. This module is about a run that already
 * happened — the durable object, read back from S3, turned into the labels and
 * groupings the Draft run tab's history renders. Pure functions only, so the
 * vocabulary the pane speaks is testable without mounting it.
 */
import type {
  DraftRunHistoryEntry,
  DraftRunHistorySection,
  DraftRunHistoryView,
  DraftRunStatus,
  RunRecordSnapshot,
  RunSectionRecords,
  SectionIntent,
} from "./tauri";
import type { StatusChipTone } from "../components/StatusChip";

/** How a run ended, as the history row states it. */
export type RunOutcome = {
  label: string;
  tone: StatusChipTone;
  /** The run is still under way, or was when the app last saw it. */
  live: boolean;
};

const RUN_OUTCOMES: Record<DraftRunStatus, RunOutcome> = {
  planning: { label: "Planning", tone: "progress", live: true },
  awaiting_approval: {
    label: "Waiting at the gate",
    tone: "warning",
    live: false,
  },
  drafting: { label: "Drafting", tone: "progress", live: true },
  stopped: { label: "Stopped", tone: "warning", live: false },
  failed: { label: "Interrupted", tone: "danger", live: false },
  completed: { label: "Completed", tone: "success", live: false },
  abandoned: { label: "Discarded", tone: "muted", live: false },
};

/**
 * How one run ended.
 *
 * `drafting` and `planning` are transient: the executor stamps `stopped` or
 * `failed` on its way out, so a run stored in either state is either running
 * in this process right now or one whose process died. The stored object
 * cannot tell those apart — a crash leaves `active_run_id` set exactly as a
 * live run does — so the caller says which run it is actually driving, and
 * everything else reads as interrupted. Saying "Drafting" about a run that
 * died months ago would be the one lie this pane exists to stop telling.
 */
export function runOutcome(
  run: DraftRunHistoryEntry,
  liveRunId: string | null,
): RunOutcome {
  const outcome = RUN_OUTCOMES[run.status];
  if (outcome.live && run.run_id !== liveRunId) {
    return { label: "Interrupted", tone: "danger", live: false };
  }
  return outcome;
}

/** One line summarising what the run decided, for the collapsed row. */
export function runSummaryLine(run: DraftRunHistoryEntry): string {
  const { counts } = run;
  const parts: string[] = [`${counts.drafted} written`];
  if (counts.kept > 0) parts.push(`${counts.kept} kept`);
  if (counts.skipped > 0) parts.push(`${counts.skipped} skipped`);
  if (counts.failed > 0) parts.push(`${counts.failed} failed`);
  if (counts.undecided > 0) parts.push(`${counts.undecided} never reached`);
  return parts.join(" · ");
}

/** Where the run's plan came from, in the words the gate uses. */
export function planOrigin(run: DraftRunHistoryEntry): string {
  if (run.planner_model_id === null) return "No plan";
  if (run.plan_synthetic) {
    return run.plan_user_edited
      ? "Built from the report's own sections, then edited"
      : "Built from the report's own sections";
  }
  return run.plan_user_edited
    ? "Planned, then edited before approval"
    : "Planned and approved unchanged";
}

/** What one section's row says happened to it. */
export type SectionOutcome = { label: string; tone: StatusChipTone };

export function sectionOutcome(
  section: DraftRunHistorySection,
): SectionOutcome {
  switch (section.state) {
    case "drafted":
      return { label: "Written", tone: "success" };
    case "flagged":
      return { label: "Written · flagged in review", tone: "warning" };
    case "kept":
      return { label: "Kept as it was", tone: "neutral" };
    case "skipped":
      return { label: "Skipped", tone: "muted" };
    case "failed":
      return { label: "Failed", tone: "danger" };
    // A section stored mid-flight belongs to a run that was interrupted; it
    // is exactly as undecided as a pending one.
    case "drafting":
    case "pending":
      return { label: "Never reached", tone: "muted" };
  }
}

const INTENT_LABELS: Record<SectionIntent, string> = {
  draft: "Draft it",
  rewrite: "Rewrite it",
  keep: "Keep what's there",
  skip: "Leave it out",
};

/** What the plan asked for this section, or `null` when no plan row named it. */
export function intentLabel(intent: SectionIntent | null): string | null {
  return intent === null ? null : INTENT_LABELS[intent];
}

/**
 * Why a section came out the way it did, in one sentence.
 *
 * A skip or a keep the plan decided is explained by the plan; a failure is
 * explained by the run's own PHI-free note. A skip nobody explained says so
 * rather than inventing a reason — the writer's own `skip_full_draft_section`
 * records no reason unless it diverged from an approved plan.
 */
export function sectionReason(section: DraftRunHistorySection): string | null {
  if (section.error !== null) return section.error;
  switch (section.state) {
    case "skipped":
      return section.intent === "skip"
        ? "The approved plan left this section out."
        : "The writer deferred this section and recorded no reason.";
    case "kept":
      return "The plan kept the previous revision's text for this section.";
    case "pending":
    case "drafting":
      return "The run ended before this section was written.";
    default:
      return null;
  }
}

/** What one section's model call was given to read. */
export type SectionRecords = {
  label: string;
  /** Filenames to list, or `null` when the reader should read the run's
   * snapshot instead of a per-section list. */
  filenames: string[] | null;
};

export function sectionRecords(
  records: RunSectionRecords | null,
  snapshot: RunRecordSnapshot | null,
): SectionRecords | null {
  if (records === null) return null;
  if (records.kind === "curated") {
    return {
      label:
        records.filenames.length === 0
          ? "Restricted to records that were no longer available"
          : `Restricted to ${countLabel(records.filenames.length, "record")}`,
      filenames: records.filenames,
    };
  }
  const included = snapshot?.files.length ?? null;
  return {
    label:
      included === null
        ? "Every readable record"
        : `Every readable record (${countLabel(included, "file")})`,
    filenames: null,
  };
}

/** `1 record` / `4 records`, so callers never build the plural inline. */
export function countLabel(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

/**
 * The run that wrote the revision on screen, if one did.
 *
 * Matched on the revision the finish cut produced rather than on recency: a
 * report can be hand-edited after a run, and a later run that was discarded
 * did not write anything the reader is looking at.
 */
export function runForRevision(
  history: DraftRunHistoryView,
  revision: number,
): DraftRunHistoryEntry | null {
  return (
    history.runs.find((run) => run.finalized_revision === revision) ?? null
  );
}

/**
 * Which run the history opens on: the one that owns the session, else the one
 * that wrote the revision on screen, else the newest.
 */
export function defaultExpandedRun(
  history: DraftRunHistoryView,
  revision: number,
): string | null {
  const active = history.runs.find((run) => run.active);
  if (active) return active.run_id;
  return (
    runForRevision(history, revision)?.run_id ?? history.runs[0]?.run_id ?? null
  );
}
