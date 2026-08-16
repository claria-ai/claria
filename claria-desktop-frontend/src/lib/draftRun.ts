import type {
  DraftRun,
  ReportContent,
  ReportSection,
  ReportTurnProgressView,
  RunSectionState,
} from "./tauri";

/**
 * What the canvas draws beside one section. Narrower than the durable
 * [`RunSectionState`]: `flagged` is a drafted section a review pass has an
 * opinion about, and the canvas has no separate treatment for it yet.
 */
export type SectionUiStatus =
  | "pending"
  | "drafting"
  | "drafted"
  | "failed"
  | "skipped"
  | "kept";

export type SectionUiState = {
  status: SectionUiStatus;
  /** Content that has landed durably, or `null` while the draft stands. */
  content: ReportSection | null;
  /** PHI-free note about why the section failed. */
  error: string | null;
};

/**
 * Everything the writer surface knows about a drafting run in flight, or
 * about one it can pick back up.
 *
 * `drafted` and `total` are copied from the progress events rather than
 * counted here: the backend knows how many sections the plan holds and how
 * many it has landed, and a dropped event must cost the reader one stale
 * frame, not a permanently wrong denominator.
 */
export type DraftRunUiState = {
  runId: string | null;
  live: boolean;
  stopping: boolean;
  total: number | null;
  drafted: number;
  title: string | null;
  sections: ReadonlyMap<string, SectionUiState>;
  /** How a run ended, read from durable state. Always `null` while live. */
  outcome: "stopped" | "failed" | null;
  /**
   * The review fan-out's own counters, kept apart from the drafting ones: a
   * review writes nothing to the report, has no sections of its own, and can
   * run over a report that never had a drafting run at all.
   */
  reviewCompleted: number;
  /** `null` until the first pass says how many properties there are. */
  reviewTotal: number | null;
  /**
   * Rows the planner has written, counted off its answer while the call is
   * still open. Kept apart from `drafted` for the same reason the review
   * counters are: planning decides what to write and writes nothing.
   */
  planned: number;
  /** `null` until the planning pass says how many sections it is planning. */
  planTotal: number | null;
};

const NO_SECTIONS: ReadonlyMap<string, SectionUiState> = new Map();

const EMPTY: DraftRunUiState = {
  runId: null,
  live: false,
  stopping: false,
  total: null,
  drafted: 0,
  title: null,
  sections: NO_SECTIONS,
  outcome: null,
  reviewCompleted: 0,
  reviewTotal: null,
  planned: 0,
  planTotal: null,
};

/** The state of a report with no drafting run anywhere near it. */
export function emptyDraftRun(): DraftRunUiState {
  return EMPTY;
}

/**
 * Durable section states, as the canvas reads them. `drafting` is transient
 * and only correct mid-run, so a section loaded in that state was interrupted
 * and is drawn as undecided.
 */
const SECTION_STATUS: Record<RunSectionState, SectionUiStatus> = {
  pending: "pending",
  drafting: "pending",
  drafted: "drafted",
  failed: "failed",
  flagged: "drafted",
  skipped: "skipped",
  kept: "kept",
};

/**
 * Hydrate the run state from a durable run — the stopped or failed run the
 * writer offers to pick back up, including after an app restart.
 */
export function runStateFromDraftRun(run: DraftRun): DraftRunUiState {
  const sections = new Map<string, SectionUiState>();
  let drafted = 0;
  for (const section of run.sections) {
    const status = SECTION_STATUS[section.state];
    if (status === "drafted") drafted += 1;
    sections.set(section.section_id, {
      status,
      content:
        status === "drafted"
          ? {
              id: section.section_id,
              heading: section.heading,
              blocks: section.blocks,
            }
          : null,
      error: section.error,
    });
  }
  return {
    runId: run.run_id,
    live: false,
    stopping: false,
    total: run.sections.length,
    drafted,
    title: run.title,
    sections,
    outcome:
      run.status === "stopped"
        ? "stopped"
        : run.status === "failed"
          ? "failed"
          : null,
    reviewCompleted: 0,
    reviewTotal: null,
    planned: 0,
    planTotal: null,
  };
}

/**
 * Fold one progress event into the run state.
 *
 * Events are fire-and-forget, so this tolerates duplicates and reordering:
 * counters only ever move forward, and a section takes the state of the last
 * event that named it. A state that changes nothing is returned unchanged, so
 * the memoized canvas does not re-render on the legacy tool events.
 */
export function reduceDraftRun(
  state: DraftRunUiState,
  event: ReportTurnProgressView
): DraftRunUiState {
  switch (event.kind) {
    // Planning never touches `live` or the section ledger, for the same
    // reason a review pass does not: it decides what to write and writes
    // nothing. The pass that started it is what put the run on screen.
    case "plan_row_planned":
      return commitPlanned(
        state,
        Math.max(state.planned, event.planned),
        event.total
      );
    case "plan_ready":
      return commit(state, {
        total: highest(state.total, event.section_count),
        drafted: state.drafted,
        title: state.title,
        sectionId: null,
        section: null,
      });
    case "section_started":
      return commit(state, {
        total: highest(state.total, event.total),
        drafted: state.drafted,
        title: state.title,
        sectionId: event.section_id,
        // Content survives: a section being rewritten shows its old text,
        // dimmed, until the new text lands.
        section: {
          status: "drafting",
          content: state.sections.get(event.section_id)?.content ?? null,
          error: null,
        },
      });
    case "section_completed":
      return commit(state, {
        total: highest(state.total, event.total),
        drafted: Math.max(state.drafted, event.drafted),
        title: state.title,
        sectionId: event.section_id,
        section: { status: "drafted", content: event.section, error: null },
      });
    case "section_skipped":
      return commit(state, {
        total: highest(state.total, event.total),
        drafted: Math.max(state.drafted, event.drafted),
        title: state.title,
        sectionId: event.section_id,
        section: { status: "skipped", content: null, error: null },
      });
    case "section_failed":
      return commit(state, {
        total: highest(state.total, event.total),
        drafted: Math.max(state.drafted, event.drafted),
        title: state.title,
        sectionId: event.section_id,
        section: {
          status: "failed",
          content: state.sections.get(event.section_id)?.content ?? null,
          error: event.message,
        },
      });
    case "title_set":
      return commit(state, {
        total: state.total,
        drafted: state.drafted,
        title: event.title,
        sectionId: null,
        section: null,
      });
    // Review passes never touch `live` or the section ledger: a review is not
    // a drafting run, has no Stop behind it, and must not make the canvas
    // draw a section bar with no sections in it.
    case "review_pass_started":
      return commitReview(state, state.reviewCompleted, event.total);
    case "review_pass_completed":
      return commitReview(
        state,
        Math.max(state.reviewCompleted, event.completed),
        event.total
      );
    default:
      // The legacy record-context, model-call, and tool events belong to the
      // qualitative activity line, not to the run's section ledger.
      return state;
  }
}

/**
 * Draw the run's landed sections over the accepted draft, so a section
 * appears in the preview the moment it is durable rather than when the whole
 * command returns. Returns the input untouched when nothing overlays it, so
 * the memoized document keeps its identity.
 */
export function overlaySections(
  content: ReportContent,
  sections: ReadonlyMap<string, SectionUiState>
): ReportContent {
  if (sections.size === 0) return content;
  let changed = false;
  const overlaid = content.sections.map((section) => {
    const landed = sections.get(section.id)?.content;
    if (!landed) return section;
    if (landed.heading === section.heading && landed.blocks === section.blocks) {
      return section;
    }
    changed = true;
    // The workspace copy still owns the template body and the authorship
    // stamp; only the words the run wrote are replaced.
    return {
      ...section,
      heading: landed.heading,
      blocks: landed.blocks,
      skipped: landed.skipped ?? false,
    };
  });
  return changed ? { ...content, sections: overlaid } : content;
}

/** Counters never move backwards, whatever order the events arrive in. */
function highest(current: number | null, incoming: number): number {
  return current === null ? incoming : Math.max(current, incoming);
}

/**
 * Branches finish out of order, so `completed` is the branch's own position
 * and the highest one wins. A round that moves nothing returns the same
 * object, so the memoized pane does not re-render on a redelivered event.
 */
function commitReview(
  state: DraftRunUiState,
  completed: number,
  total: number
): DraftRunUiState {
  const reviewTotal = highest(state.reviewTotal, total);
  if (state.reviewCompleted === completed && state.reviewTotal === reviewTotal) {
    return state;
  }
  return { ...state, reviewCompleted: completed, reviewTotal };
}

/**
 * The planner's row count, which restarts from nothing whenever its call is
 * re-sent, so the highest one wins and the reader never watches it fall back.
 */
function commitPlanned(
  state: DraftRunUiState,
  planned: number,
  total: number
): DraftRunUiState {
  const planTotal = highest(state.planTotal, total);
  if (state.planned === planned && state.planTotal === planTotal) return state;
  return { ...state, planned, planTotal };
}

function commit(
  state: DraftRunUiState,
  fields: {
    total: number | null;
    drafted: number;
    title: string | null;
    sectionId: string | null;
    section: SectionUiState | null;
  }
): DraftRunUiState {
  const sectionChanged =
    fields.sectionId !== null &&
    fields.section !== null &&
    !sameSectionState(state.sections.get(fields.sectionId), fields.section);

  if (
    state.live &&
    state.outcome === null &&
    state.total === fields.total &&
    state.drafted === fields.drafted &&
    state.title === fields.title &&
    !sectionChanged
  ) {
    return state;
  }

  const sections =
    sectionChanged && fields.sectionId !== null && fields.section !== null
      ? new Map(state.sections).set(fields.sectionId, fields.section)
      : state.sections;

  return {
    ...state,
    // Any section-level event means a run is under way, which retires
    // whatever outcome an earlier run left behind.
    live: true,
    outcome: null,
    total: fields.total,
    drafted: fields.drafted,
    title: fields.title,
    sections,
  };
}

function sameSectionState(
  current: SectionUiState | undefined,
  incoming: SectionUiState
): boolean {
  return (
    current !== undefined &&
    current.status === incoming.status &&
    current.error === incoming.error &&
    sameSection(current.content, incoming.content)
  );
}

/**
 * Structural comparison, so a redelivered `section_completed` carrying a
 * fresh object with identical content is recognised as the duplicate it is.
 */
function sameSection(
  current: ReportSection | null,
  incoming: ReportSection | null
): boolean {
  if (current === incoming) return true;
  if (current === null || incoming === null) return false;
  return JSON.stringify(current) === JSON.stringify(incoming);
}
