// The session flow diagram's PHI posture is the point under test: with the
// toggle off (the default on every mount) no filename, summary, or message
// text may exist in the DOM — only names, counts, sizes, and statuses.

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { ReportAuthoringTurnView, TurnUsage } from "../lib/tauri";
import {
  buildChatSessionDiagram,
  buildWriterSessionDiagram,
} from "../lib/sessionDiagram";
import SessionFlowDiagram from "./SessionFlowDiagram";

const MODEL = "us.anthropic.claude-sonnet-test";

const USAGE: TurnUsage = {
  model_id: MODEL,
  input_tokens: 3,
  output_tokens: 60,
  cache_read_input_tokens: 4_243,
  cache_write_input_tokens: 5_000,
  cache_ttl: "five_minutes",
  cost_usd: 0.021,
  pricing_version: 1,
};

const WRITER_TURN: ReportAuthoringTurnView = {
  id: "turn-a",
  model_id: MODEL,
  timeline: [
    {
      kind: "message",
      role: "user",
      text: "Summarize the intake note for Emma Watson",
      created_at: "2026-08-11T12:00:00Z",
    },
    {
      kind: "tool_activity",
      name: "read_record",
      summary: "Read intake-note.pdf",
      status: "succeeded",
      invocation_json: '{"filename":"intake-note.pdf"}',
      result_json: '{"text":"chart content"}',
      created_at: "2026-08-11T12:00:02Z",
    },
    {
      kind: "tool_activity",
      name: "replace_section",
      summary: "Replace section History",
      status: "failed",
      invocation_json: '{"section_id":"s1"}',
      result_json: null,
      created_at: "2026-08-11T12:00:04Z",
    },
    {
      kind: "message",
      role: "assistant",
      text: "I proposed a summary of the intake note.",
      created_at: "2026-08-11T12:00:07Z",
    },
  ],
  usage: USAGE,
  usage_complete: true,
  converse_calls: 3,
  tool_uses: 2,
  context_files: [{ filename: "intake-note.pdf", available: true }],
  context_reads: [],
  created_at: "2026-08-11T12:00:00Z",
  completed_at: "2026-08-11T12:00:08Z",
};

const PHI_STRINGS = [
  "Emma Watson", // instruction excerpt
  "intake-note.pdf", // record filename (context node + tool summary)
  "Read intake-note.pdf", // tool summary
  "I proposed a summary", // reply excerpt
];

describe("SessionFlowDiagram", () => {
  it("renders no PHI by default, only names, counts, sizes, and statuses", () => {
    const { container } = render(
      <SessionFlowDiagram model={buildWriterSessionDiagram([WRITER_TURN])} />
    );

    const text = container.textContent ?? "";
    for (const phi of PHI_STRINGS) {
      expect(text).not.toContain(phi);
    }

    // The PHI-free skeleton is all there.
    expect(screen.getByText("Turn 1", { exact: false })).toBeDefined();
    expect(screen.getByText("read_record")).toBeDefined();
    expect(screen.getByText("replace_section")).toBeDefined();
    expect(screen.getByText("failed")).toBeDefined();
    expect(screen.getByText("9,246 tok context · 60 tok output")).toBeDefined();
    expect(screen.getByText("3 model calls · 2 tool uses")).toBeDefined();
    expect(screen.getByText("8.0s")).toBeDefined();
    expect(screen.getByText("$0.021 · Claude Sonnet Test")).toBeDefined();
    expect(screen.getByText("1 file")).toBeDefined();
    expect(
      screen.getByText(`call ${'{"section_id":"s1"}'.length} chars · no result`)
    ).toBeDefined();
  });

  it("reveals excerpts, summaries, and filenames when PHI is opted in", async () => {
    const { container } = render(
      <SessionFlowDiagram model={buildWriterSessionDiagram([WRITER_TURN])} />
    );

    const toggle = screen.getByLabelText("Include PHI");
    expect((toggle as HTMLInputElement).checked).toBe(false);
    await userEvent.click(toggle);

    const text = container.textContent ?? "";
    for (const phi of PHI_STRINGS) {
      expect(text).toContain(phi);
    }
  });

  it("flags max_tokens stops on chat turns", () => {
    const model = buildChatSessionDiagram([
      { role: "user", content: "Q", timestamp: "2026-08-11T12:00:00Z" },
      {
        role: "assistant",
        content: "A",
        timestamp: "2026-08-11T12:00:02Z",
        usage: USAGE,
        stopReason: "max_tokens",
      },
    ]);
    render(<SessionFlowDiagram model={model} />);
    expect(screen.getByText("Stopped early: hit max tokens")).toBeDefined();
    // Chat's state lane carries the derived prompt-cache activity.
    expect(screen.getByText("read 4,243 tok")).toBeDefined();
    expect(screen.getByText("wrote 5,000 tok")).toBeDefined();
  });

  it("shows its own empty state and disables the PHI toggle without turns", () => {
    render(<SessionFlowDiagram model={{ surface: "chat", turns: [] }} />);
    expect(screen.getByText("No turns yet")).toBeDefined();
    const toggle = screen.getByLabelText("Include PHI") as HTMLInputElement;
    expect(toggle.disabled).toBe(true);
  });
});
