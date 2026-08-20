import { describe, expect, it } from "vitest";

import {
  defaultExpandedRun,
  intentLabel,
  planOrigin,
  runForRevision,
  runOutcome,
  runSummaryLine,
  sectionOutcome,
  sectionReason,
  sectionRecords,
} from "./draftRunHistory";
import type {
  DraftRunHistoryEntry,
  DraftRunHistorySection,
  RunRecordSnapshot,
} from "./tauri";

const SECTION_ID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

function section(
  overrides: Partial<DraftRunHistorySection> = {},
): DraftRunHistorySection {
  return {
    section_id: SECTION_ID,
    heading: "Background",
    position: 0,
    state: "drafted",
    intent: "draft",
    required: true,
    scope: "State the developmental history.",
    evidence: [],
    instruction: null,
    records: { kind: "shared_corpus" },
    citations: [],
    block_count: 3,
    characters: 900,
    attempts: 1,
    error: null,
    updated_at: "2026-08-18T02:00:00Z",
    ...overrides,
  };
}

function run(
  overrides: Partial<DraftRunHistoryEntry> = {},
): DraftRunHistoryEntry {
  return {
    run_id: "run-1",
    status: "completed",
    active: false,
    base_revision: 1,
    finalized_revision: 2,
    partial: false,
    title: "Psychoeducational Evaluation",
    writer_model_id: "us.anthropic.claude-opus-test",
    planner_model_id: "us.anthropic.claude-sonnet-test",
    plan_synthetic: false,
    plan_user_edited: false,
    plan_approved_at: "2026-08-18T01:00:00Z",
    plan_warnings: [],
    instructions: [],
    record_snapshot: null,
    sections: [section()],
    counts: {
      total: 3,
      drafted: 3,
      skipped: 0,
      kept: 0,
      failed: 0,
      undecided: 0,
      decided: 3,
    },
    created_at: "2026-08-18T01:00:00Z",
    updated_at: "2026-08-18T02:00:00Z",
    ...overrides,
  };
}

const snapshot: RunRecordSnapshot = {
  files: [
    { filename: "intake.pdf", sha256: "a".repeat(64), characters: 4200 },
    { filename: "teacher-form.pdf", sha256: "b".repeat(64), characters: 1100 },
  ],
  unavailable: [],
  total_characters: 5300,
  captured_at: "2026-08-18T01:00:00Z",
};

describe("runOutcome", () => {
  it("names each terminal status", () => {
    expect(runOutcome(run({ status: "completed" }), null).label).toBe(
      "Completed",
    );
    expect(runOutcome(run({ status: "stopped" }), null).label).toBe("Stopped");
    expect(runOutcome(run({ status: "abandoned" }), null).label).toBe(
      "Discarded",
    );
  });

  it("keeps the run this app is driving live", () => {
    const live = runOutcome(run({ status: "drafting" }), "run-1");
    expect(live.label).toBe("Drafting");
    expect(live.live).toBe(true);
  });

  // A crash leaves `active_run_id` set exactly as a live run does, so holding
  // the session is not evidence that anything is still running.
  it("reads a stranded drafting run as interrupted even while it holds the session", () => {
    const stranded = runOutcome(
      run({ status: "drafting", active: true }),
      null,
    );
    expect(stranded.label).toBe("Interrupted");
    expect(stranded.live).toBe(false);
  });
});

describe("runSummaryLine", () => {
  it("names only the outcomes that happened", () => {
    expect(runSummaryLine(run())).toBe("3 written");
  });

  it("counts every kind a mixed run produced", () => {
    const mixed = run({
      counts: {
        total: 6,
        drafted: 2,
        skipped: 1,
        kept: 1,
        failed: 1,
        undecided: 1,
        decided: 5,
      },
    });
    expect(runSummaryLine(mixed)).toBe(
      "2 written · 1 kept · 1 skipped · 1 failed · 1 never reached",
    );
  });
});

describe("planOrigin", () => {
  it("separates a planned run from a manufactured one", () => {
    expect(planOrigin(run())).toBe("Planned and approved unchanged");
    expect(planOrigin(run({ plan_user_edited: true }))).toBe(
      "Planned, then edited before approval",
    );
    expect(planOrigin(run({ plan_synthetic: true }))).toBe(
      "Built from the report's own sections",
    );
    expect(planOrigin(run({ planner_model_id: null }))).toBe("No plan");
  });
});

describe("sectionOutcome and sectionReason", () => {
  it("labels each durable section state", () => {
    expect(sectionOutcome(section()).label).toBe("Written");
    expect(sectionOutcome(section({ state: "kept" })).label).toBe(
      "Kept as it was",
    );
    expect(sectionOutcome(section({ state: "failed" })).label).toBe("Failed");
    expect(sectionOutcome(section({ state: "pending" })).label).toBe(
      "Never reached",
    );
    // Transient on disk means the run died mid-section, not that it is going.
    expect(sectionOutcome(section({ state: "drafting" })).label).toBe(
      "Never reached",
    );
  });

  it("prefers the run's own note over an inferred reason", () => {
    expect(
      sectionReason(
        section({ state: "failed", error: "no_supporting_records" }),
      ),
    ).toBe("no_supporting_records");
  });

  it("distinguishes a planned skip from an unexplained one", () => {
    expect(sectionReason(section({ state: "skipped", intent: "skip" }))).toBe(
      "The approved plan left this section out.",
    );
    expect(sectionReason(section({ state: "skipped", intent: "draft" }))).toBe(
      "The writer deferred this section and recorded no reason.",
    );
  });

  it("says nothing about a section that simply worked", () => {
    expect(sectionReason(section())).toBeNull();
  });
});

describe("sectionRecords", () => {
  it("points a shared-corpus section at the run's snapshot", () => {
    const records = sectionRecords({ kind: "shared_corpus" }, snapshot);
    expect(records?.label).toBe("Every readable record (2 files)");
    expect(records?.filenames).toBeNull();
  });

  it("lists the files a restricted section actually got", () => {
    const records = sectionRecords(
      { kind: "curated", filenames: ["teacher-form.pdf"] },
      snapshot,
    );
    expect(records?.label).toBe("Restricted to 1 record");
    expect(records?.filenames).toEqual(["teacher-form.pdf"]);
  });

  it("says so when a restriction reached nothing at all", () => {
    const records = sectionRecords(
      { kind: "curated", filenames: [] },
      snapshot,
    );
    expect(records?.label).toBe(
      "Restricted to records that were no longer available",
    );
  });

  it("has nothing to say about a section no model was asked about", () => {
    expect(sectionRecords(null, snapshot)).toBeNull();
  });
});

describe("intentLabel", () => {
  it("reads the plan's directive back in the gate's words", () => {
    expect(intentLabel("keep")).toBe("Keep what's there");
    expect(intentLabel(null)).toBeNull();
  });
});

describe("choosing which run to show", () => {
  const older = run({ run_id: "run-0", finalized_revision: 1 });
  const newer = run({ run_id: "run-1", finalized_revision: 2 });
  const history = { runs: [newer, older], active_run_id: null };

  it("matches on the revision a run cut, not on recency", () => {
    expect(runForRevision(history, 1)?.run_id).toBe("run-0");
    expect(runForRevision(history, 99)).toBeNull();
  });

  it("opens the run that wrote the revision on screen", () => {
    expect(defaultExpandedRun(history, 1)).toBe("run-0");
  });

  it("prefers the run that owns the session over any other", () => {
    const held = {
      runs: [newer, { ...older, active: true }],
      active_run_id: "run-0",
    };
    expect(defaultExpandedRun(held, 2)).toBe("run-0");
  });

  it("falls back to the newest run when nothing else applies", () => {
    expect(defaultExpandedRun(history, 42)).toBe("run-1");
    expect(defaultExpandedRun({ runs: [], active_run_id: null }, 1)).toBeNull();
  });
});
