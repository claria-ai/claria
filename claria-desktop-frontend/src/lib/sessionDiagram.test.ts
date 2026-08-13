import { describe, expect, it } from "vitest";
import type {
  ReportAuthoringTurnView,
  ReportProposalResolution,
  ReportTimelineItemView,
  TurnUsage,
} from "./tauri";
import {
  buildChatSessionDiagram,
  buildWriterSessionDiagram,
  durationBetween,
  formatChars,
  formatDurationMs,
  phiExcerpt,
  type AssistantMessageNode,
  type CacheActivityNode,
  type ContextFilesNode,
  type ModelUsageNode,
  type ProposalDecisionNode,
  type ToolActivityNode,
  type UserMessageNode,
} from "./sessionDiagram";

const MODEL = "us.anthropic.claude-sonnet-test";

function usage(over: Partial<TurnUsage> = {}): TurnUsage {
  return {
    model_id: MODEL,
    input_tokens: 3,
    output_tokens: 60,
    cache_read_input_tokens: 4_243,
    cache_write_input_tokens: 5_000,
    cache_ttl: "five_minutes",
    cost_usd: 0.021,
    pricing_version: 1,
    ...over,
  };
}

function writerTurn(
  over: Partial<ReportAuthoringTurnView> = {}
): ReportAuthoringTurnView {
  return {
    id: "turn-a",
    model_id: MODEL,
    timeline: [],
    usage: usage(),
    usage_complete: true,
    converse_calls: 3,
    tool_uses: 2,
    context_files: [],
    context_reads: [],
    created_at: "2026-08-11T12:00:00Z",
    completed_at: "2026-08-11T12:00:08Z",
    ...over,
  };
}

const USER_TEXT = "Summarize the intake note for Emma Watson";

function userItem(text: string = USER_TEXT): ReportTimelineItemView {
  return {
    kind: "message",
    role: "user",
    text,
    created_at: "2026-08-11T12:00:00Z",
  };
}

const USER_ITEM = userItem();

const TOOL_OK: ReportTimelineItemView = {
  kind: "tool_activity",
  name: "read_record",
  summary: "Read intake-note.pdf",
  status: "succeeded",
  invocation_json: '{"filename":"intake-note.pdf","offset":0}',
  result_json: '{"text":"chart content"}',
  created_at: "2026-08-11T12:00:02Z",
};

const TOOL_FAILED: ReportTimelineItemView = {
  kind: "tool_activity",
  name: "replace_section",
  summary: "Replace section History",
  status: "failed",
  invocation_json: '{"section_id":"s1"}',
  result_json: null,
  created_at: "2026-08-11T12:00:04Z",
};

const ASSISTANT_ITEM: ReportTimelineItemView = {
  kind: "message",
  role: "assistant",
  text: "I read the intake note and proposed a summary.",
  created_at: "2026-08-11T12:00:07Z",
};

describe("buildWriterSessionDiagram", () => {
  it("maps a writer timeline onto lanes in order, with the turn tally on the reply", () => {
    const model = buildWriterSessionDiagram([
      writerTurn({
        timeline: [USER_ITEM, TOOL_OK, TOOL_FAILED, ASSISTANT_ITEM],
        context_files: [
          { filename: "intake-note.pdf", available: true },
          { filename: "labs.csv", available: false },
        ],
      }),
    ]);

    expect(model.surface).toBe("writer");
    expect(model.turns).toHaveLength(1);
    const turn = model.turns[0];
    expect(turn.number).toBe(1);
    expect(turn.timestamp).toBe("2026-08-11T12:00:00Z");
    expect(turn.nodes.map((node) => node.kind)).toEqual([
      "user_message",
      "context_files",
      "tool_activity",
      "tool_activity",
      "assistant_message",
    ]);
    expect(turn.nodes.map((node) => node.lane)).toEqual([
      "user",
      "state",
      "state",
      "state",
      "llm",
    ]);

    const user = turn.nodes[0] as UserMessageNode;
    expect(user.chars).toBe(USER_TEXT.length);
    expect(user.phiExcerpt).toBe(USER_TEXT);

    const context = turn.nodes[1] as ContextFilesNode;
    expect(context.fileCount).toBe(2);
    expect(context.unavailableCount).toBe(1);
    expect(context.phiFilenames).toEqual(["intake-note.pdf", "labs.csv"]);

    const toolOk = turn.nodes[2] as ToolActivityNode;
    expect(toolOk.toolName).toBe("read_record");
    expect(toolOk.status).toBe("succeeded");
    expect(toolOk.invocationChars).toBe(
      '{"filename":"intake-note.pdf","offset":0}'.length
    );
    expect(toolOk.resultChars).toBe('{"text":"chart content"}'.length);
    expect(toolOk.phiSummary).toBe("Read intake-note.pdf");

    const toolFailed = turn.nodes[3] as ToolActivityNode;
    expect(toolFailed.status).toBe("failed");
    expect(toolFailed.resultChars).toBeNull();

    const reply = turn.nodes[4] as AssistantMessageNode;
    expect(reply.tally).not.toBeNull();
    expect(reply.tally?.contextTokens).toBe(3 + 4_243 + 5_000);
    expect(reply.tally?.freshInputTokens).toBe(3);
    expect(reply.tally?.cacheReadTokens).toBe(4_243);
    expect(reply.tally?.cacheWriteTokens).toBe(5_000);
    expect(reply.tally?.outputTokens).toBe(60);
    expect(reply.tally?.costUsd).toBe(0.021);
    expect(reply.tally?.costKnown).toBe(true);
    expect(reply.tally?.converseCalls).toBe(3);
    expect(reply.tally?.toolUses).toBe(2);
    expect(reply.tally?.durationMs).toBe(8_000);
    expect(reply.tally?.stopReason).toBeNull();
  });

  it("falls back to a model-usage node when a turn has no assistant message", () => {
    const model = buildWriterSessionDiagram([
      writerTurn({ timeline: [USER_ITEM, TOOL_OK], usage_complete: false }),
    ]);
    const nodes = model.turns[0].nodes;
    expect(nodes.map((node) => node.kind)).toEqual([
      "user_message",
      "tool_activity",
      "model_usage",
    ]);
    const usageNode = nodes[2] as ModelUsageNode;
    expect(usageNode.tally.contextTokens).toBe(3 + 4_243 + 5_000);
    expect(usageNode.tally.usageComplete).toBe(false);
  });

  it("truncates PHI excerpts to the excerpt budget", () => {
    const long = "chart note ".repeat(20).trim(); // 219 chars
    const model = buildWriterSessionDiagram([
      writerTurn({ timeline: [userItem(long)] }),
    ]);
    const user = model.turns[0].nodes[0] as UserMessageNode;
    expect(user.chars).toBe(long.length);
    expect(user.phiExcerpt?.endsWith("…")).toBe(true);
    expect(user.phiExcerpt?.length).toBeLessThanOrEqual(81);
  });

  it("attaches resolutions to the turn the user decided in and drops trimmed ones", () => {
    const turnOne = writerTurn({
      id: "turn-1",
      created_at: "2026-08-11T12:00:00Z",
      timeline: [USER_ITEM, ASSISTANT_ITEM],
    });
    const turnTwo = writerTurn({
      id: "turn-2",
      created_at: "2026-08-11T12:10:00Z",
      completed_at: "2026-08-11T12:10:09Z",
      timeline: [USER_ITEM, ASSISTANT_ITEM],
    });
    const resolutions: ReportProposalResolution[] = [
      {
        proposal_id: "p-accepted",
        decision: "accepted",
        resulting_revision: 5,
        resolved_at: "2026-08-11T12:05:00Z",
      },
      {
        proposal_id: "p-rejected",
        decision: "rejected",
        resulting_revision: 5,
        resolved_at: "2026-08-11T12:11:00Z",
      },
      {
        proposal_id: "p-trimmed",
        decision: "accepted",
        resulting_revision: 1,
        resolved_at: "2026-08-11T11:00:00Z",
      },
    ];

    const model = buildWriterSessionDiagram([turnOne, turnTwo], resolutions);

    const firstDecisions = model.turns[0].nodes.filter(
      (node): node is ProposalDecisionNode => node.kind === "proposal_decision"
    );
    expect(firstDecisions).toHaveLength(1);
    expect(firstDecisions[0].decision).toBe("accepted");
    expect(firstDecisions[0].resultingRevision).toBe(5);
    expect(firstDecisions[0].lane).toBe("user");

    const secondDecisions = model.turns[1].nodes.filter(
      (node): node is ProposalDecisionNode => node.kind === "proposal_decision"
    );
    expect(secondDecisions).toHaveLength(1);
    expect(secondDecisions[0].decision).toBe("rejected");
  });
});

describe("buildChatSessionDiagram", () => {
  it("groups exchanges into turns with cache activity in the state lane", () => {
    const model = buildChatSessionDiagram([
      {
        role: "user",
        content: "Question about the chart",
        timestamp: "2026-08-11T12:00:00Z",
      },
      {
        role: "assistant",
        content: "Answer",
        timestamp: "2026-08-11T12:00:02Z",
        usage: usage(),
        stopReason: "max_tokens",
      },
      {
        role: "user",
        content: "Follow-up",
        timestamp: "2026-08-11T12:01:00Z",
      },
      {
        role: "assistant",
        content: "Legacy answer",
        timestamp: null,
        usage: null,
      },
    ]);

    expect(model.surface).toBe("chat");
    expect(model.turns).toHaveLength(2);
    expect(model.turns.map((turn) => turn.number)).toEqual([1, 2]);

    const first = model.turns[0];
    expect(first.nodes.map((node) => node.kind)).toEqual([
      "user_message",
      "cache_activity",
      "assistant_message",
    ]);
    const cache = first.nodes[1] as CacheActivityNode;
    expect(cache.lane).toBe("state");
    expect(cache.readTokens).toBe(4_243);
    expect(cache.writeTokens).toBe(5_000);
    const reply = first.nodes[2] as AssistantMessageNode;
    expect(reply.tally?.stopReason).toBe("max_tokens");
    expect(reply.tally?.durationMs).toBe(2_000);
    expect(reply.tally?.converseCalls).toBeNull();
    expect(reply.tally?.toolUses).toBeNull();

    // Legacy turn: no usage means no cache node and no tally.
    const second = model.turns[1];
    expect(second.nodes.map((node) => node.kind)).toEqual([
      "user_message",
      "assistant_message",
    ]);
    expect((second.nodes[1] as AssistantMessageNode).tally).toBeNull();
  });

  it("marks costs from an unknown pricing table as unknown", () => {
    const model = buildChatSessionDiagram([
      { role: "user", content: "Q" },
      {
        role: "assistant",
        content: "A",
        usage: usage({ pricing_version: 0, cost_usd: 0 }),
      },
    ]);
    const reply = model.turns[0].nodes.at(-1) as AssistantMessageNode;
    expect(reply.tally?.costKnown).toBe(false);
  });

  it("returns no turns for an empty transcript", () => {
    expect(buildChatSessionDiagram([]).turns).toEqual([]);
  });
});

describe("formatting helpers", () => {
  it("formats character counts like token counts", () => {
    expect(formatChars(532)).toBe("532 chars");
    expect(formatChars(12_345)).toBe("12.3k chars");
    expect(formatChars(null)).toBe("—");
  });

  it("formats durations", () => {
    expect(formatDurationMs(2_400)).toBe("2.4s");
    expect(formatDurationMs(72_000)).toBe("1m 12s");
  });

  it("computes durations only when both endpoints parse in order", () => {
    expect(
      durationBetween("2026-08-11T12:00:00Z", "2026-08-11T12:00:08Z")
    ).toBe(8_000);
    expect(durationBetween(null, "2026-08-11T12:00:08Z")).toBeNull();
    expect(durationBetween("not a date", "2026-08-11T12:00:08Z")).toBeNull();
    expect(
      durationBetween("2026-08-11T12:00:08Z", "2026-08-11T12:00:00Z")
    ).toBeNull();
  });

  it("collapses whitespace and truncates excerpts", () => {
    expect(phiExcerpt("  a   b\n\nc ")).toBe("a b c");
    expect(phiExcerpt("   ")).toBeNull();
    const truncated = phiExcerpt("x".repeat(200));
    expect(truncated).toBe(`${"x".repeat(80)}…`);
  });
});
