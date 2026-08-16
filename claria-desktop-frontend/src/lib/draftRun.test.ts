import { describe, expect, it } from "vitest";
import {
  emptyDraftRun,
  overlaySections,
  reduceDraftRun,
  runStateFromDraftRun,
  type DraftRunUiState,
  type SectionUiState,
} from "./draftRun";
import type {
  DraftRun,
  ReportContent,
  ReportSection,
  ReportTurnProgressView,
  RunSection,
  RunSectionState,
} from "./tauri";

const REFERRAL = "11111111-1111-4111-8111-111111111111";
const BACKGROUND = "22222222-2222-4222-8222-222222222222";
const SUMMARY = "33333333-3333-4333-8333-333333333333";

function section(id: string, heading: string, text: string): ReportSection {
  return { id, heading, blocks: [{ kind: "paragraph", text }] };
}

function apply(
  events: ReportTurnProgressView[],
  from: DraftRunUiState = emptyDraftRun()
): DraftRunUiState {
  return events.reduce(reduceDraftRun, from);
}

function runSection(
  sectionId: string,
  heading: string,
  state: RunSectionState,
  overrides: Partial<RunSection> = {}
): RunSection {
  return {
    section_id: sectionId,
    heading,
    position: 0,
    state,
    blocks: [],
    citations: [],
    attempts: 1,
    error: null,
    updated_at: "2026-08-01T00:00:00Z",
    ...overrides,
  };
}

function draftRun(overrides: Partial<DraftRun> = {}): DraftRun {
  return {
    schema_version: 1,
    run_id: "99999999-9999-4999-8999-999999999999",
    report_id: "88888888-8888-4888-8888-888888888888",
    client_id: "77777777-7777-4777-8777-777777777777",
    base_revision: 3,
    status: "stopped",
    plan: null,
    title: "Psychoeducational Evaluation",
    sections: [],
    instructions: [],
    writer_model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0",
    finalized_revision: null,
    partial: false,
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:10:00Z",
    ...overrides,
  };
}

/** One clean run of three sections, in the order the backend emits them. */
const ORDERED: ReportTurnProgressView[] = [
  { kind: "plan_ready", section_count: 3 },
  { kind: "title_set", title: "Psychoeducational Evaluation" },
  { kind: "section_started", section_id: REFERRAL, index: 0, total: 3 },
  {
    kind: "section_completed",
    section_id: REFERRAL,
    section: section(REFERRAL, "Reason for Referral", "Referred for attention."),
    drafted: 1,
    total: 3,
  },
  { kind: "section_started", section_id: BACKGROUND, index: 1, total: 3 },
  { kind: "section_skipped", section_id: BACKGROUND, drafted: 1, total: 3 },
  { kind: "section_started", section_id: SUMMARY, index: 2, total: 3 },
  {
    kind: "section_failed",
    section_id: SUMMARY,
    message: "section_attempts_exhausted",
    drafted: 1,
    total: 3,
  },
];

describe("emptyDraftRun", () => {
  it("describes a report with no run near it", () => {
    const empty = emptyDraftRun();
    expect(empty).toEqual({
      runId: null,
      live: false,
      stopping: false,
      total: null,
      drafted: 0,
      title: null,
      sections: new Map(),
      outcome: null,
    });
  });

  it("returns one shared value, so a reset is a cheap identity change", () => {
    expect(emptyDraftRun()).toBe(emptyDraftRun());
  });
});

describe("reduceDraftRun", () => {
  it("folds an ordered run into counters, a title, and per-section state", () => {
    const state = apply(ORDERED);

    expect(state.live).toBe(true);
    expect(state.outcome).toBeNull();
    expect(state.total).toBe(3);
    expect(state.drafted).toBe(1);
    expect(state.title).toBe("Psychoeducational Evaluation");
    expect(state.sections.get(REFERRAL)).toEqual({
      status: "drafted",
      content: section(REFERRAL, "Reason for Referral", "Referred for attention."),
      error: null,
    });
    expect(state.sections.get(BACKGROUND)).toEqual({
      status: "skipped",
      content: null,
      error: null,
    });
    expect(state.sections.get(SUMMARY)).toEqual({
      status: "failed",
      content: null,
      error: "section_attempts_exhausted",
    });
  });

  it("takes the plan's section count as the denominator before any section starts", () => {
    const state = apply([{ kind: "plan_ready", section_count: 9 }]);
    expect(state.total).toBe(9);
    expect(state.drafted).toBe(0);
    expect(state.live).toBe(true);
  });

  it("leaves the state untouched for the legacy tool and context events", () => {
    const started = apply([{ kind: "plan_ready", section_count: 3 }]);
    const legacy: ReportTurnProgressView[] = [
      {
        kind: "record_context_prepared",
        included_files: 3,
        unavailable_files: 0,
        total_characters: 6400,
      },
      { kind: "model_call_started", call_number: 2 },
      { kind: "tool_started", name: "write_full_draft_section", context: null },
      {
        kind: "tool_finished",
        name: "write_full_draft_section",
        context: null,
        status: "succeeded",
      },
    ];
    for (const event of legacy) {
      expect(reduceDraftRun(started, event)).toBe(started);
    }
  });

  it("returns the same object when the event that set the state repeats", () => {
    const state = apply(ORDERED);
    // Every event that still describes the state it produced: the counters,
    // the title, and the last word on each section.
    const redelivered: ReportTurnProgressView[] = [
      ORDERED[0],
      ORDERED[1],
      ORDERED[3],
      ORDERED[5],
      ORDERED[7],
    ];
    for (const event of redelivered) {
      expect(reduceDraftRun(state, event)).toBe(state);
    }
  });

  it("recognises a redelivered section as a duplicate despite a fresh object", () => {
    const state = apply(ORDERED);
    const redelivered: ReportTurnProgressView = {
      kind: "section_completed",
      section_id: REFERRAL,
      // Structurally identical, but a different object than the one that
      // landed — which is exactly what a redelivered channel event looks like.
      section: section(REFERRAL, "Reason for Referral", "Referred for attention."),
      drafted: 1,
      total: 3,
    };
    expect(reduceDraftRun(state, redelivered)).toBe(state);
  });

  it("never lets counters move backwards", () => {
    const state = apply([
      { kind: "plan_ready", section_count: 9 },
      {
        kind: "section_completed",
        section_id: REFERRAL,
        section: section(REFERRAL, "Reason for Referral", "Written."),
        drafted: 4,
        total: 9,
      },
      // A stale event overtaken in flight.
      { kind: "section_started", section_id: BACKGROUND, index: 0, total: 2 },
      { kind: "section_skipped", section_id: SUMMARY, drafted: 1, total: 2 },
    ]);

    expect(state.total).toBe(9);
    expect(state.drafted).toBe(4);
  });

  it("keeps a section's existing content while it is being rewritten", () => {
    const state = apply([
      ...ORDERED,
      { kind: "section_started", section_id: REFERRAL, index: 0, total: 3 },
    ]);

    expect(state.sections.get(REFERRAL)).toEqual({
      status: "drafting",
      content: section(REFERRAL, "Reason for Referral", "Referred for attention."),
      error: null,
    });
  });

  it("keeps the last state a section was given, whatever order the events arrive in", () => {
    const state = apply([
      { kind: "plan_ready", section_count: 3 },
      {
        kind: "section_completed",
        section_id: REFERRAL,
        section: section(REFERRAL, "Reason for Referral", "Written."),
        drafted: 1,
        total: 3,
      },
      {
        kind: "section_failed",
        section_id: REFERRAL,
        message: "section_attempts_exhausted",
        drafted: 1,
        total: 3,
      },
    ]);

    const referral = state.sections.get(REFERRAL);
    expect(referral?.status).toBe("failed");
    expect(referral?.error).toBe("section_attempts_exhausted");
    // The words it did write are still there to show under the failure.
    expect(referral?.content).toEqual(
      section(REFERRAL, "Reason for Referral", "Written.")
    );
  });

  it("clears a failure when the section is written on the next attempt", () => {
    const state = apply([
      { kind: "plan_ready", section_count: 1 },
      {
        kind: "section_failed",
        section_id: REFERRAL,
        message: "section_attempts_exhausted",
        drafted: 0,
        total: 1,
      },
      {
        kind: "section_completed",
        section_id: REFERRAL,
        section: section(REFERRAL, "Reason for Referral", "Second try."),
        drafted: 1,
        total: 1,
      },
    ]);

    expect(state.sections.get(REFERRAL)?.error).toBeNull();
    expect(state.sections.get(REFERRAL)?.status).toBe("drafted");
  });

  it("retires a stopped run's outcome as soon as the next run reports in", () => {
    const stopped = runStateFromDraftRun(
      draftRun({
        status: "stopped",
        sections: [runSection(REFERRAL, "Reason for Referral", "pending")],
      })
    );
    expect(stopped.outcome).toBe("stopped");

    const resumed = reduceDraftRun(stopped, {
      kind: "section_started",
      section_id: REFERRAL,
      index: 0,
      total: 1,
    });
    expect(resumed.outcome).toBeNull();
    expect(resumed.live).toBe(true);
    // The run identity survives, so the banner's actions still know it.
    expect(resumed.runId).toBe(stopped.runId);
  });
});

describe("runStateFromDraftRun", () => {
  it("maps every durable section state onto what the canvas draws", () => {
    const state = runStateFromDraftRun(
      draftRun({
        status: "stopped",
        sections: [
          runSection(REFERRAL, "Reason for Referral", "drafted", {
            blocks: [{ kind: "paragraph", text: "Referred for attention." }],
          }),
          runSection(BACKGROUND, "Background", "skipped"),
          runSection(SUMMARY, "Summary", "failed", {
            error: "section_attempts_exhausted",
          }),
          runSection("44444444-4444-4444-8444-444444444444", "History", "kept"),
          runSection("55555555-5555-4555-8555-555555555555", "Testing", "pending"),
          runSection("66666666-6666-4666-8666-666666666666", "Scores", "flagged", {
            blocks: [{ kind: "paragraph", text: "Scores are within range." }],
          }),
        ],
      })
    );

    expect(state.total).toBe(6);
    expect(state.drafted).toBe(2);
    expect(state.outcome).toBe("stopped");
    expect(state.live).toBe(false);
    expect(state.stopping).toBe(false);
    expect(state.title).toBe("Psychoeducational Evaluation");
    expect(state.runId).toBe("99999999-9999-4999-8999-999999999999");

    expect(state.sections.get(REFERRAL)).toEqual({
      status: "drafted",
      content: section(REFERRAL, "Reason for Referral", "Referred for attention."),
      error: null,
    });
    expect(state.sections.get(BACKGROUND)?.status).toBe("skipped");
    expect(state.sections.get(SUMMARY)).toEqual({
      status: "failed",
      content: null,
      error: "section_attempts_exhausted",
    });
    expect(state.sections.get("44444444-4444-4444-8444-444444444444")?.status).toBe(
      "kept"
    );
    expect(state.sections.get("55555555-5555-4555-8555-555555555555")?.status).toBe(
      "pending"
    );
    // A flagged section is drafted; the canvas has no separate mark for it.
    expect(state.sections.get("66666666-6666-4666-8666-666666666666")?.status).toBe(
      "drafted"
    );
  });

  it("draws a section interrupted mid-write as undecided", () => {
    const state = runStateFromDraftRun(
      draftRun({
        sections: [runSection(REFERRAL, "Reason for Referral", "drafting")],
      })
    );
    expect(state.sections.get(REFERRAL)?.status).toBe("pending");
    expect(state.sections.get(REFERRAL)?.content).toBeNull();
    expect(state.drafted).toBe(0);
  });

  it("reads the failed outcome off a failed run", () => {
    expect(runStateFromDraftRun(draftRun({ status: "failed" })).outcome).toBe(
      "failed"
    );
  });

  it("reports no outcome for a run that is still going", () => {
    expect(runStateFromDraftRun(draftRun({ status: "drafting" })).outcome).toBeNull();
  });
});

describe("overlaySections", () => {
  const content: ReportContent = {
    title: "Untitled report",
    sections: [
      section(REFERRAL, "Reason for Referral", "Template boilerplate."),
      section(BACKGROUND, "Background", "Template boilerplate."),
    ],
  };

  it("returns the content itself when nothing has landed", () => {
    expect(overlaySections(content, new Map())).toBe(content);
  });

  it("returns the content itself when the landed sections match it already", () => {
    const sections = new Map<string, SectionUiState>([
      [REFERRAL, { status: "drafted", content: content.sections[0], error: null }],
    ]);
    expect(overlaySections(content, sections)).toBe(content);
  });

  it("draws landed sections over the accepted draft by section id", () => {
    const landed = section(REFERRAL, "Reason for Referral", "Referred for attention.");
    const sections = new Map<string, SectionUiState>([
      [REFERRAL, { status: "drafted", content: landed, error: null }],
      [BACKGROUND, { status: "drafting", content: null, error: null }],
    ]);

    const overlaid = overlaySections(content, sections);
    expect(overlaid).not.toBe(content);
    expect(overlaid.title).toBe("Untitled report");
    expect(overlaid.sections[0].blocks).toEqual(landed.blocks);
    // A section still being written keeps the words that are on the page.
    expect(overlaid.sections[1]).toBe(content.sections[1]);
  });

  it("keeps the workspace's template copy under a landed section", () => {
    const withTemplate: ReportContent = {
      title: "Untitled report",
      sections: [
        {
          ...section(REFERRAL, "Reason for Referral", "Template boilerplate."),
          skipped: true,
          template_blocks: [{ kind: "paragraph", text: "Template boilerplate." }],
        },
      ],
    };
    const sections = new Map<string, SectionUiState>([
      [
        REFERRAL,
        {
          status: "drafted",
          content: section(REFERRAL, "Reason for Referral", "Written now."),
          error: null,
        },
      ],
    ]);

    const overlaid = overlaySections(withTemplate, sections);
    expect(overlaid.sections[0].template_blocks).toEqual([
      { kind: "paragraph", text: "Template boilerplate." },
    ]);
    // Writing into a skipped section un-skips it.
    expect(overlaid.sections[0].skipped).toBe(false);
    expect(overlaid.sections[0].blocks).toEqual([
      { kind: "paragraph", text: "Written now." },
    ]);
  });

  it("ignores landed sections the report does not have", () => {
    const sections = new Map<string, SectionUiState>([
      [
        "00000000-0000-4000-8000-000000000000",
        { status: "drafted", content: section(SUMMARY, "Summary", "Orphan."), error: null },
      ],
    ]);
    expect(overlaySections(content, sections)).toBe(content);
  });
});
