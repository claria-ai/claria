import { describe, expect, it, vi } from "vitest";

vi.mock("./tauri", () => ({
  getPrompt: vi.fn(async () => ""),
  listWriterLibraryPrompts: vi.fn(async () => []),
  listWriterTemplates: vi.fn(async () => []),
}));
vi.mock("./logBridge", () => ({ logFrontendEvent: vi.fn() }));

import { searchUserContent, type UserContentEntry } from "./preferencesSearchContent";

const entries: UserContentEntry[] = [
  {
    paneId: "prompts.chat-system",
    title: "Chat System Prompt text",
    text: "You are a clinical assistant helping a psychologist set up a new client record.",
  },
  {
    paneId: "writer.prompt-library",
    title: 'Saved prompt "Phase 1"',
    text: "Phase 1\nFill in the Reason for Referral section.",
  },
];

describe("searchUserContent", () => {
  it("returns nothing for an empty query", () => {
    expect(searchUserContent(entries, "")).toEqual([]);
  });

  it("matches inside the body and produces a snippet around the match", () => {
    const hits = searchUserContent(entries, "psychologist");
    expect(hits).toHaveLength(1);
    expect(hits[0].paneId).toBe("prompts.chat-system");
    expect(hits[0].snippet).toContain("psychologist");
    expect(hits[0].context).toBe("Prompts › Chat System Prompt");
  });

  it("matches on the entry title too", () => {
    const hits = searchUserContent(entries, "phase 1");
    expect(hits.some((h) => h.paneId === "writer.prompt-library")).toBe(true);
  });

  it("elides long bodies around the match", () => {
    const long: UserContentEntry = {
      paneId: "writer.prompt",
      title: "Writer Prompt text",
      text: `${"a".repeat(200)} needle ${"b".repeat(200)}`,
    };
    const [hit] = searchUserContent([long], "needle");
    expect(hit.snippet).toContain("…");
    expect(hit.snippet.length).toBeLessThan(120);
  });
});
