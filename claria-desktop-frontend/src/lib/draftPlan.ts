import type {
  DraftRun,
  EvidenceRef,
  PlanEntryEdit,
  RunSectionState,
  SectionIntent,
} from "./tauri";

/**
 * Which gate the reader is standing at. A fresh plan is a decision about a
 * report nothing has been written into yet; a resume is a decision about one
 * that is already part-written, so it offers two more directives and defaults
 * every row from what the interrupted run actually landed.
 */
export type PlanGateKind = "fresh" | "resume";

/**
 * What the Draft run pane is for right now. The two gates ask a question and
 * wait; the running mode answers it, showing the same plan read-only above
 * the progress bar.
 */
export type DraftPlanMode = "plan-gate" | "resume-gate" | "running";

/** The directives a fresh plan may choose between. */
export const FRESH_INTENTS: readonly SectionIntent[] = ["draft", "skip"];

/** The directives a resumed run may choose between, in escalating order. */
export const RESUME_INTENTS: readonly SectionIntent[] = [
  "keep",
  "rewrite",
  "draft",
  "skip",
];

export const INTENT_LABELS: Record<SectionIntent, string> = {
  draft: "Draft",
  rewrite: "Rewrite",
  keep: "Keep",
  skip: "Skip",
};

/** One editable row of the plan, as the gate shows it. */
export type PlanRow = {
  sectionId: string;
  heading: string;
  intent: SectionIntent;
  required: boolean;
  scope: string;
  evidence: EvidenceRef[];
  /** Empty string rather than null: this is a text-input buffer. */
  instruction: string;
  /** What the interrupted run left this section in. `null` at a fresh gate. */
  priorState: RunSectionState | null;
  /** PHI-free note about why the section failed, when one did. */
  priorError: string | null;
};

/**
 * What a resumed run should do with a section, before the reader touches it.
 *
 * These are the same defaults the backend derives when a run is picked back
 * up with no new instructions, so a reader who changes nothing gets exactly
 * what the deterministic resume would have done anyway.
 */
export function resumeIntentFor(state: RunSectionState): SectionIntent {
  switch (state) {
    case "drafted":
    case "flagged":
    case "kept":
      return "keep";
    case "skipped":
      return "skip";
    default:
      return "draft";
  }
}

/**
 * The plan's rows in document order.
 *
 * `run.sections` is the authority on which sections exist and where they sit;
 * the plan supplies what the planner decided about each. A run whose plan has
 * not landed yet — or one interrupted before it ever had one — still produces
 * a complete, editable row per section.
 */
export function planRows(run: DraftRun, kind: PlanGateKind): PlanRow[] {
  const entries = new Map(
    (run.plan?.entries ?? []).map((entry) => [entry.section_id, entry])
  );
  return [...run.sections]
    .sort((left, right) => left.position - right.position)
    .map((section) => {
      const entry = entries.get(section.section_id);
      return {
        sectionId: section.section_id,
        heading: entry?.heading ?? section.heading,
        intent:
          kind === "resume"
            ? resumeIntentFor(section.state)
            : (entry?.intent ?? "draft"),
        required: entry?.required ?? true,
        scope: entry?.scope ?? "",
        evidence: entry?.evidence ?? [],
        instruction: entry?.instruction ?? "",
        priorState: kind === "resume" ? section.state : null,
        priorError: section.error,
      };
    });
}

/** Sections the plan will put words into: everything it does not skip. */
export function plannedSectionCount(rows: readonly PlanRow[]): number {
  return rows.filter((row) => row.intent !== "skip").length;
}

/** Sections a resumed run still has to write. */
export function remainingSectionCount(rows: readonly PlanRow[]): number {
  return rows.filter((row) => row.intent === "draft" || row.intent === "rewrite")
    .length;
}

/**
 * The named-field patches for the rows the reader actually changed.
 *
 * One row contributes one patch carrying only its own edited fields, so a
 * gate that touched a single scope never restates the planner's evidence,
 * and a gate that touched nothing sends no request at all.
 */
export function changedPlanEdits(
  original: readonly PlanRow[],
  current: readonly PlanRow[]
): PlanEntryEdit[] {
  const before = new Map(original.map((row) => [row.sectionId, row]));
  const edits: PlanEntryEdit[] = [];
  for (const row of current) {
    const was = before.get(row.sectionId);
    if (!was) continue;
    const edit: PlanEntryEdit = { section_id: row.sectionId };
    let changed = false;
    if (was.intent !== row.intent) {
      edit.intent = row.intent;
      changed = true;
    }
    if (was.scope !== row.scope) {
      edit.scope = row.scope;
      changed = true;
    }
    if (!sameEvidence(was.evidence, row.evidence)) {
      edit.evidence = row.evidence;
      changed = true;
    }
    if (was.instruction !== row.instruction) {
      edit.instruction = row.instruction;
      changed = true;
    }
    if (changed) edits.push(edit);
  }
  return edits;
}

function sameEvidence(
  left: readonly EvidenceRef[],
  right: readonly EvidenceRef[]
): boolean {
  return (
    left.length === right.length &&
    left.every(
      (item, index) =>
        item.filename === right[index].filename &&
        (item.note ?? null) === (right[index].note ?? null)
    )
  );
}
