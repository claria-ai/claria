import { describe, expect, it } from "vitest";
import {
  categoryOf,
  defaultOpenPanes,
  paneSpec,
  searchPreferencesNav,
  PREFERENCES_NAV,
  WRITER_FOCUS_PANES,
} from "./preferencesNav";

describe("PREFERENCES_NAV integrity", () => {
  it("pane ids are unique across all categories", () => {
    const ids = PREFERENCES_NAV.flatMap((c) => c.panes.map((p) => p.id));
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("category ids are unique", () => {
    const ids = PREFERENCES_NAV.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("every pane id prefixes its category id", () => {
    for (const category of PREFERENCES_NAV) {
      for (const pane of category.panes) {
        expect(pane.id.startsWith(`${category.id}.`)).toBe(true);
      }
    }
  });

  it("field anchors are unique within each pane", () => {
    for (const category of PREFERENCES_NAV) {
      for (const pane of category.panes) {
        const anchors = pane.fields.map((f) => f.anchor);
        expect(new Set(anchors).size).toBe(anchors.length);
      }
    }
  });

  it("paneSpec and categoryOf resolve every pane", () => {
    for (const category of PREFERENCES_NAV) {
      for (const pane of category.panes) {
        expect(paneSpec(pane.id)).toBe(pane);
        expect(categoryOf(pane.id)).toBe(category.id);
      }
    }
  });

  it("the Writing-tab jump targets exist", () => {
    expect(categoryOf(WRITER_FOCUS_PANES["writer-templates"])).toBe("writer");
    expect(categoryOf(WRITER_FOCUS_PANES["writer-prompts"])).toBe("writer");
  });

  it("each category starts with at least one open pane", () => {
    const open = new Set(defaultOpenPanes());
    for (const category of PREFERENCES_NAV) {
      expect(category.panes.some((pane) => open.has(pane.id))).toBe(true);
    }
  });
});

describe("searchPreferencesNav", () => {
  it("returns nothing for an empty or whitespace query", () => {
    expect(searchPreferencesNav("")).toEqual([]);
    expect(searchPreferencesNav("   ")).toEqual([]);
  });

  it("finds a field through a synonym", () => {
    const hits = searchPreferencesNav("diarization");
    const speakers = hits.find((h) => h.anchor === "default_speaker_count");
    expect(speakers).toBeTruthy();
    expect(speakers?.paneId).toBe("transcription.imported-audio");
    expect(speakers?.kind).toBe("field");
  });

  it("is case-insensitive and finds every temperature control", () => {
    const hits = searchPreferencesNav("TEMPERATURE");
    const paneIds = hits.map((h) => h.paneId);
    expect(paneIds).toContain("claude.model-tuning");
    expect(paneIds).toContain("transcription.local-decoding");
  });

  it("ranks a pane-title match ahead of synonym matches", () => {
    const hits = searchPreferencesNav("model tuning");
    expect(hits[0]).toMatchObject({
      kind: "pane",
      paneId: "claude.model-tuning",
    });
  });

  it("matches a category by title", () => {
    const hits = searchPreferencesNav("billing");
    expect(hits[0]).toMatchObject({ kind: "category", categoryId: "billing" });
  });

  it("finds the on-device models pane via 'whisper'", () => {
    const hits = searchPreferencesNav("whisper");
    expect(
      hits.some((h) => h.paneId === "transcription.local-models")
    ).toBe(true);
  });

  it("finds auto-lock by the words a clinician would use", () => {
    for (const query of ["pin", "touch id", "idle", "lock"]) {
      expect(
        searchPreferencesNav(query).some(
          (hit) => hit.paneId === "security.auto-lock"
        )
      ).toBe(true);
    }
  });

  it("field hits carry a breadcrumb with category and pane titles", () => {
    const hit = searchPreferencesNav("hourly data resolution")[0];
    expect(hit.context).toBe("Billing › Cost Explorer");
  });
});
