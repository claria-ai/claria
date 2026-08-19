import { describe, expect, it } from "vitest";
import type { DraftRun } from "./tauri";
import {
  changedPlanEdits,
  emptyRestrictionCount,
  plannedSectionCount,
  planRows,
  remainingSectionCount,
} from "./draftPlan";

const A = "11111111-1111-4111-8111-111111111111";
const B = "22222222-2222-4222-8222-222222222222";

function run(overrides: Partial<DraftRun> = {}): DraftRun {
  return {
    schema_version: 1,
    run_id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
    report_id: "report-1",
    client_id: "client-1",
    base_revision: 0,
    status: "awaiting_approval",
    title: null,
    plan: {
      model_id: "planner-1",
      user_edited: false,
      approved_at: null,
      plan_warnings: [],
      created_at: "2026-08-01T00:00:00Z",
      entries: [
        {
          section_id: A,
          heading: "Findings",
          intent: "draft",
          required: true,
          scope: "Assessment results.",
          evidence: [{ filename: "intake.txt", note: null }],
          instruction: null,
        },
        {
          section_id: B,
          heading: "Summary",
          intent: "skip",
          required: false,
          scope: "",
          evidence: [],
          instruction: null,
        },
      ],
    },
    sections: [
      section(B, "Summary", 1, "pending"),
      section(A, "Findings", 0, "pending"),
    ],
    instructions: [],
    writer_model_id: "model-1",
    finalized_revision: null,
    partial: false,
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
    ...overrides,
  };
}

function section(
  id: string,
  heading: string,
  position: number,
  state: DraftRun["sections"][number]["state"],
  error: string | null = null
): DraftRun["sections"][number] {
  return {
    section_id: id,
    heading,
    position,
    state,
    blocks: [],
    citations: [],
    attempts: 0,
    error,
    updated_at: "2026-08-01T00:00:00Z",
  };
}

describe("planRows", () => {
  it("orders rows by the document position, not by plan order", () => {
    expect(planRows(run(), "fresh").map((row) => row.heading)).toEqual([
      "Findings",
      "Summary",
    ]);
  });

  it("takes a fresh gate's directives from the plan", () => {
    expect(planRows(run(), "fresh").map((row) => row.intent)).toEqual([
      "draft",
      "skip",
    ]);
  });

  it("takes a resumed gate's directives from what the run landed", () => {
    const stopped = run({
      status: "stopped",
      sections: [
        section(A, "Findings", 0, "drafted"),
        section(B, "Summary", 1, "failed", "section_attempts_exhausted"),
      ],
    });
    const rows = planRows(stopped, "resume");
    expect(rows.map((row) => row.intent)).toEqual(["keep", "draft"]);
    expect(rows[1].priorState).toBe("failed");
    expect(rows[1].priorError).toBe("section_attempts_exhausted");
    // Scope and evidence still come from the plan the run was made against.
    expect(rows[0].scope).toBe("Assessment results.");
  });

  it("still produces a full row per section when a run has no plan", () => {
    const rows = planRows(run({ plan: null }), "fresh");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({ heading: "Findings", intent: "draft", scope: "" });
  });
});

describe("section counts", () => {
  it("counts everything the plan does not skip", () => {
    expect(plannedSectionCount(planRows(run(), "fresh"))).toBe(1);
  });

  it("counts only what a resumed run still has to write", () => {
    const stopped = run({
      status: "stopped",
      sections: [
        section(A, "Findings", 0, "drafted"),
        section(B, "Summary", 1, "pending"),
      ],
    });
    expect(remainingSectionCount(planRows(stopped, "resume"))).toBe(1);
  });
});

describe("changedPlanEdits", () => {
  it("sends nothing when nothing was touched", () => {
    const rows = planRows(run(), "fresh");
    expect(changedPlanEdits(rows, rows)).toEqual([]);
  });

  it("sends one patch per changed row, carrying only its changed fields", () => {
    const rows = planRows(run(), "fresh");
    const edited = [
      { ...rows[0], scope: "Only the cognitive results.", evidence: [] },
      { ...rows[1], intent: "draft" as const },
    ];
    expect(changedPlanEdits(rows, edited)).toEqual([
      { section_id: A, scope: "Only the cognitive results.", evidence: [] },
      { section_id: B, intent: "draft" },
    ]);
  });

  it("treats equal evidence lists as unchanged", () => {
    const rows = planRows(run(), "fresh");
    const same = [
      { ...rows[0], evidence: [{ filename: "intake.txt", note: null }] },
      rows[1],
    ];
    expect(changedPlanEdits(rows, same)).toEqual([]);
  });
});

describe("curated records", () => {
  it("reads an unrestricted plan as null rather than an empty list", () => {
    expect(planRows(run(), "fresh").map((row) => row.curatedRecords)).toEqual([
      null,
      null,
    ]);
  });

  it("carries a restriction the plan already holds onto its row", () => {
    const restricted = run();
    restricted.plan!.entries[0].curated_records = ["intake.txt"];
    expect(planRows(restricted, "fresh")[0].curatedRecords).toEqual([
      "intake.txt",
    ]);
  });

  it("patches a new restriction as its own field", () => {
    const rows = planRows(run(), "fresh");
    const edited = [{ ...rows[0], curatedRecords: ["intake.txt"] }, rows[1]];
    expect(changedPlanEdits(rows, edited)).toEqual([
      { section_id: A, curated_records: ["intake.txt"] },
    ]);
  });

  // `null` and `[]` mean the same thing to the backend — no restriction — so
  // clearing one has to travel as the empty list rather than as nothing.
  it("clears a restriction by sending an empty list", () => {
    const restricted = run();
    restricted.plan!.entries[0].curated_records = ["intake.txt"];
    const rows = planRows(restricted, "fresh");
    const edited = [{ ...rows[0], curatedRecords: null }, rows[1]];
    expect(changedPlanEdits(rows, edited)).toEqual([
      { section_id: A, curated_records: [] },
    ]);
  });

  it("treats an unchanged restriction as unchanged", () => {
    const restricted = run();
    restricted.plan!.entries[0].curated_records = ["intake.txt", "teacher.txt"];
    const rows = planRows(restricted, "fresh");
    const same = [
      { ...rows[0], curatedRecords: ["intake.txt", "teacher.txt"] },
      rows[1],
    ];
    expect(changedPlanEdits(rows, same)).toEqual([]);
  });

  it("counts a restriction switched on but left empty, and nothing else", () => {
    const rows = planRows(run(), "fresh");
    expect(emptyRestrictionCount(rows)).toBe(0);
    expect(
      emptyRestrictionCount([{ ...rows[0], curatedRecords: [] }, rows[1]])
    ).toBe(1);
    expect(
      emptyRestrictionCount([
        { ...rows[0], curatedRecords: ["intake.txt"] },
        rows[1],
      ])
    ).toBe(0);
    // A skipped section is not going to be drafted at all, so an empty
    // restriction on one blocks nothing.
    expect(
      emptyRestrictionCount([rows[0], { ...rows[1], curatedRecords: [] }])
    ).toBe(0);
  });
});
