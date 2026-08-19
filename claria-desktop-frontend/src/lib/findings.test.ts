import { describe, expect, it } from "vitest";
import {
  anchoredQuote,
  conflictingHeading,
  currentSectionRevision,
  findingIsStale,
  findingState,
  openFindingCounts,
  reviewPropertyLabel,
  summarizeCompletion,
} from "./findings";
import type { CompletionReport, Finding, ReportContent } from "./tauri";

const FINDINGS_ID = "11111111-1111-4111-8111-111111111111";
const BACKGROUND_ID = "22222222-2222-4222-8222-222222222222";

function content(overrides: Partial<ReportContent> = {}): ReportContent {
  return {
    title: "Evaluation",
    sections: [
      {
        id: BACKGROUND_ID,
        heading: "Background",
        blocks: [{ kind: "paragraph", text: "Jane is eleven years old." }],
      },
      {
        id: FINDINGS_ID,
        heading: "Findings",
        blocks: [
          { kind: "bullet_list", items: ["A point"] },
          { kind: "paragraph", text: "Jane presents as attentive today." },
        ],
      },
    ],
    ...overrides,
  };
}

function finding(overrides: Partial<Finding> = {}): Finding {
  return {
    id: "f-1",
    pass: "style",
    property: "tense_drift",
    model_id: "model-1",
    anchor: { section_id: FINDINGS_ID, revision: 2 },
    description: "Tense slips.",
    span: null,
    conflicting: null,
    record_citation: null,
    proposal: null,
    status: "open",
    applied_revision: null,
    resolved_at: null,
    created_at: "2026-08-01T00:00:00Z",
    ...overrides,
  };
}

describe("reviewPropertyLabel", () => {
  it("reads the backend's property name as prose", () => {
    expect(reviewPropertyLabel("cross_section_conflict")).toBe(
      "cross section conflict"
    );
  });
});

describe("findingIsStale", () => {
  it("treats an unstamped section as fresh", () => {
    expect(findingIsStale(finding(), content())).toBe(false);
  });

  it("goes stale once the section's authorship passes the anchor", () => {
    const stamped = content();
    stamped.sections[1].authorship = {
      kind: "model_revised",
      revision: 3,
      model_id: "model-1",
      run_id: null,
      updated_at: "2026-08-02T00:00:00Z",
    };
    expect(findingIsStale(finding(), stamped)).toBe(true);

    stamped.sections[1].authorship.revision = 2;
    expect(findingIsStale(finding(), stamped)).toBe(false);
  });

  it("treats a deleted section as stale", () => {
    const gone = content();
    gone.sections = gone.sections.filter(
      (section) => section.id !== FINDINGS_ID
    );
    expect(findingIsStale(finding(), gone)).toBe(true);
  });
});

describe("findingState", () => {
  it("lets a resolution outrank staleness so the undo stays reachable", () => {
    const stamped = content();
    stamped.sections[1].authorship = {
      kind: "human_edited",
      revision: 9,
      model_id: null,
      run_id: null,
      updated_at: "2026-08-02T00:00:00Z",
    };

    expect(
      findingState(
        finding({ status: "applied", applied_revision: 3 }),
        stamped
      )
    ).toBe("applied");
    expect(findingState(finding({ status: "dismissed" }), stamped)).toBe(
      "dismissed"
    );
    expect(findingState(finding(), stamped)).toBe("invalidated");
    expect(findingState(finding(), content())).toBe("open");
  });

  it("honours the stored invalidated stamp the list path writes", () => {
    expect(findingState(finding({ status: "invalidated" }), content())).toBe(
      "invalidated"
    );
  });
});

describe("openFindingCounts", () => {
  it("counts only the open findings, per section", () => {
    const counts = openFindingCounts(
      [
        finding({ id: "a" }),
        finding({ id: "b" }),
        finding({ id: "c", status: "dismissed" }),
        finding({
          id: "d",
          anchor: { section_id: BACKGROUND_ID, revision: 1 },
        }),
      ],
      content()
    );

    expect(counts.get(FINDINGS_ID)).toBe(2);
    expect(counts.get(BACKGROUND_ID)).toBe(1);
  });
});

describe("anchoredQuote", () => {
  it("slices the passage out of the anchored paragraph", () => {
    expect(
      anchoredQuote(
        finding({ span: { block_index: 1, start_char: 5, end_char: 22 } }),
        content()
      )
    ).toBe("presents as atten");
  });

  it("returns nothing rather than inventing a quote", () => {
    expect(anchoredQuote(finding(), content())).toBeNull();
    // Block 0 is a bullet list, which has no single text to slice.
    expect(
      anchoredQuote(
        finding({ span: { block_index: 0, start_char: 0, end_char: 4 } }),
        content()
      )
    ).toBeNull();
  });
});

describe("conflictingHeading", () => {
  it("names the section a consistency finding contradicts", () => {
    expect(
      conflictingHeading(
        finding({
          conflicting: { section_id: BACKGROUND_ID, quote: "nine", span: null },
        }),
        content()
      )
    ).toBe("Background");
    expect(
      conflictingHeading(
        finding({
          conflicting: { section_id: null, quote: "nine", span: null },
        }),
        content()
      )
    ).toBeNull();
  });
});

describe("currentSectionRevision", () => {
  it("prefers the section's own stamp and falls back to the draft", () => {
    const stamped = content();
    stamped.sections[1].authorship = {
      kind: "human_edited",
      revision: 6,
      model_id: null,
      run_id: null,
      updated_at: "2026-08-02T00:00:00Z",
    };

    expect(currentSectionRevision(finding(), stamped, 8)).toBe(6);
    expect(currentSectionRevision(finding(), content(), 8)).toBe(8);
  });
});

describe("summarizeCompletion", () => {
  function report(checks: CompletionReport["checks"]): CompletionReport {
    return {
      complete: checks.length === 0,
      checks,
      evaluated_revision: 4,
      evaluated_at: "2026-08-02T00:00:00Z",
    };
  }

  it("counts each kind once and names the sections it can", () => {
    const rows = summarizeCompletion(
      report([
        {
          kind: "unresolved_finding",
          section_id: FINDINGS_ID,
          detail: "open:2",
        },
        {
          kind: "unresolved_finding",
          section_id: BACKGROUND_ID,
          detail: "open:1",
        },
        {
          kind: "unresolved_finding",
          section_id: FINDINGS_ID,
          detail: "open:3",
        },
        {
          kind: "unresolved_citation",
          section_id: FINDINGS_ID,
          detail: "intake.txt",
        },
        {
          kind: "unresolved_citation",
          section_id: FINDINGS_ID,
          detail: "scores.json",
        },
        { kind: "placeholder_text", section_id: null, detail: "title" },
      ]),
      content()
    );

    expect(rows.map((row) => row.label)).toEqual([
      "1 section still has placeholder text",
      "2 citations could not be verified",
      "3 open findings",
    ]);
    expect(rows[2].sections).toEqual(["Findings", "Background"]);
    // A document-wide failure counts without naming anything.
    expect(rows[0].sections).toEqual([]);
  });

  it("has nothing to say about a complete report", () => {
    expect(summarizeCompletion(report([]), content())).toEqual([]);
  });
});
