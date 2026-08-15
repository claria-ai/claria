// Pure builders for the three-lane session flow diagram shown in the
// usage panel: user actions, model responses, and Claria state changes,
// grouped by turn with PHI-free tallies (token counts, costs, sizes,
// statuses, durations). PHI-bearing strings (excerpts, tool summaries,
// filenames) are carried on dedicated `phi*` fields so the renderer can
// keep them out of the DOM entirely unless the user opts in.
//
// Never persisted, never fetched — derived on render from data that
// already crosses IPC (writer turn views, chat messages).

import type {
  ChatRole,
  ReportAuthoringTurnView,
  ReportProposalDecision,
  ReportProposalResolution,
  ReportToolActivityStatus,
  TurnUsage,
} from "./tauri";

/** Which vertical lane a node renders in. */
export type DiagramLane = "user" | "llm" | "state";

/** Longest PHI excerpt the diagram may carry (characters). */
export const PHI_EXCERPT_MAX_CHARS = 80;

/**
 * Per-turn usage tallies attached to the turn's LLM-lane node. All fields
 * are PHI-free numbers/identifiers. `contextTokens` is the context the
 * model actually processed: fresh + cache read + cache write.
 */
export interface DiagramTurnTally {
  modelId: string;
  freshInputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  outputTokens: number;
  contextTokens: number;
  /** The frozen audit-of-record per-turn cost. */
  costUsd: number;
  /** False when the pricing table was unknown (`pricing_version` 0). */
  costKnown: boolean;
  /** False for writer turns whose usage capture was cut short. */
  usageComplete: boolean;
  /** Writer only — Converse round-trips this turn. `null` for chat. */
  converseCalls: number | null;
  /** Writer only — tool invocations this turn. `null` for chat. */
  toolUses: number | null;
  /** Chat only — final stop reason of the streamed response, when seen. */
  stopReason: string | null;
  /** Wall-clock turn duration when both endpoints are known. */
  durationMs: number | null;
}

export interface UserMessageNode {
  kind: "user_message";
  lane: "user";
  id: string;
  timestamp: string | null;
  chars: number;
  /** Truncated instruction excerpt — render only when PHI is opted in. */
  phiExcerpt: string | null;
}

export interface ProposalDecisionNode {
  kind: "proposal_decision";
  lane: "user";
  id: string;
  timestamp: string;
  decision: ReportProposalDecision;
  resultingRevision: number;
}

export interface AssistantMessageNode {
  kind: "assistant_message";
  lane: "llm";
  id: string;
  timestamp: string | null;
  chars: number;
  /** Truncated reply excerpt — render only when PHI is opted in. */
  phiExcerpt: string | null;
  /** Present on the turn's final assistant message. */
  tally: DiagramTurnTally | null;
}

/** Fallback LLM-lane node for a metered turn with no assistant message. */
export interface ModelUsageNode {
  kind: "model_usage";
  lane: "llm";
  id: string;
  timestamp: string | null;
  tally: DiagramTurnTally;
}

export interface ToolActivityNode {
  kind: "tool_activity";
  lane: "state";
  id: string;
  timestamp: string | null;
  toolName: string;
  status: ReportToolActivityStatus;
  /** Size of the model's invocation payload, in characters. */
  invocationChars: number;
  /** Size of the tool result payload; `null` while/if no result exists. */
  resultChars: number | null;
  /** Truncated activity summary — render only when PHI is opted in. */
  phiSummary: string | null;
}

/** Records Claria preloaded into the model's context this turn. */
export interface ContextFilesNode {
  kind: "context_files";
  lane: "state";
  id: string;
  timestamp: string | null;
  fileCount: number;
  unavailableCount: number;
  /** Record filenames — render only when PHI is opted in. */
  phiFilenames: string[];
}

/** Chat-only state node derived from usage: prompt-cache read/write. */
export interface CacheActivityNode {
  kind: "cache_activity";
  lane: "state";
  id: string;
  timestamp: string | null;
  readTokens: number;
  writeTokens: number;
}

export type DiagramNode =
  | UserMessageNode
  | ProposalDecisionNode
  | AssistantMessageNode
  | ModelUsageNode
  | ToolActivityNode
  | ContextFilesNode
  | CacheActivityNode;

export interface DiagramTurn {
  id: string;
  /** 1-based display number. */
  number: number;
  /** When the turn started, for the group separator. */
  timestamp: string | null;
  nodes: DiagramNode[];
}

export interface SessionDiagramModel {
  surface: "chat" | "writer";
  turns: DiagramTurn[];
}

// ---------------------------------------------------------------------------
// Writer builder
// ---------------------------------------------------------------------------

/**
 * Build the diagram model from writer turn views plus the workspace's
 * proposal resolutions. Resolutions are attached (by `resolved_at`) to the
 * latest turn that had started when the user decided; resolutions older
 * than the first retained turn are dropped with their trimmed turn.
 */
export function buildWriterSessionDiagram(
  turns: ReadonlyArray<ReportAuthoringTurnView>,
  resolutions: ReadonlyArray<ReportProposalResolution> = []
): SessionDiagramModel {
  const diagramTurns = turns.map((turn, turnIndex) => {
    const nodes: DiagramNode[] = [];
    let nodeIndex = 0;
    const nextId = () => `${turn.id}-n${nodeIndex++}`;

    const contextNode: ContextFilesNode | null =
      turn.context_files.length > 0
        ? {
            kind: "context_files",
            lane: "state",
            id: `${turn.id}-context`,
            timestamp: turn.created_at,
            fileCount: turn.context_files.length,
            unavailableCount: turn.context_files.filter(
              (file) => !file.available
            ).length,
            phiFilenames: turn.context_files.map((file) => file.filename),
          }
        : null;
    let contextPlaced = contextNode === null;

    for (const item of turn.timeline) {
      if (item.kind === "message") {
        if (item.role === "user") {
          nodes.push({
            kind: "user_message",
            lane: "user",
            id: nextId(),
            timestamp: item.created_at,
            chars: item.text.length,
            phiExcerpt: phiExcerpt(item.text),
          });
          if (!contextPlaced && contextNode) {
            nodes.push(contextNode);
            contextPlaced = true;
          }
        } else {
          nodes.push({
            kind: "assistant_message",
            lane: "llm",
            id: nextId(),
            timestamp: item.created_at,
            chars: item.text.length,
            phiExcerpt: phiExcerpt(item.text),
            tally: null,
          });
        }
      } else {
        if (!contextPlaced && contextNode) {
          nodes.push(contextNode);
          contextPlaced = true;
        }
        nodes.push({
          kind: "tool_activity",
          lane: "state",
          id: nextId(),
          timestamp: item.created_at,
          toolName: item.name,
          status: item.status,
          invocationChars: item.invocation_json.length,
          resultChars: item.result_json?.length ?? null,
          phiSummary: phiExcerpt(item.summary),
        });
      }
    }
    if (!contextPlaced && contextNode) nodes.push(contextNode);

    const tally = buildTally(turn.usage, {
      usageComplete: turn.usage_complete,
      converseCalls: turn.converse_calls,
      toolUses: turn.tool_uses,
      stopReason: null,
      durationMs: durationBetween(turn.created_at, turn.completed_at),
    });
    const lastReply = [...nodes]
      .reverse()
      .find((node): node is AssistantMessageNode => node.kind === "assistant_message");
    if (lastReply) {
      lastReply.tally = tally;
    } else {
      nodes.push({
        kind: "model_usage",
        lane: "llm",
        id: `${turn.id}-usage`,
        timestamp: turn.completed_at,
        tally,
      });
    }

    return {
      id: turn.id,
      number: turnIndex + 1,
      timestamp: turn.created_at,
      nodes,
    } satisfies DiagramTurn;
  });

  attachResolutions(diagramTurns, turns, resolutions);
  return { surface: "writer", turns: diagramTurns };
}

/**
 * Append each resolution as a user-lane decision node on the latest turn
 * whose `created_at` is not after `resolved_at`. Unparseable decision
 * times fall back to the last turn; decisions predating every retained
 * turn are dropped.
 */
function attachResolutions(
  diagramTurns: DiagramTurn[],
  turns: ReadonlyArray<ReportAuthoringTurnView>,
  resolutions: ReadonlyArray<ReportProposalResolution>
): void {
  if (diagramTurns.length === 0) return;
  const turnStarts = turns.map((turn) => Date.parse(turn.created_at));
  const ordered = [...resolutions].sort(
    (a, b) => Date.parse(a.resolved_at) - Date.parse(b.resolved_at)
  );
  for (const resolution of ordered) {
    const resolvedAt = Date.parse(resolution.resolved_at);
    let target = diagramTurns.length - 1;
    if (Number.isFinite(resolvedAt)) {
      target = -1;
      for (let i = 0; i < turnStarts.length; i++) {
        if (Number.isFinite(turnStarts[i]) && turnStarts[i] <= resolvedAt) {
          target = i;
        }
      }
      if (target === -1) continue; // its turn is no longer retained
    }
    diagramTurns[target].nodes.push({
      kind: "proposal_decision",
      lane: "user",
      id: `${diagramTurns[target].id}-r${resolution.proposal_id}`,
      timestamp: resolution.resolved_at,
      decision: resolution.decision,
      resultingRevision: resolution.resulting_revision,
    });
  }
}

// ---------------------------------------------------------------------------
// Chat builder
// ---------------------------------------------------------------------------

/** One chat message with the per-index metadata the chat widget tracks. */
export interface ChatDiagramMessage {
  role: ChatRole;
  content: string;
  timestamp?: string | null;
  usage?: TurnUsage | null;
  /** Stop reason from the streamed `done` event, when the caller saw one. */
  stopReason?: string | null;
}

/**
 * Build the diagram model from an ordered chat transcript. Each user
 * message starts a turn. Chat has no tool timeline, so its state lane
 * carries only prompt-cache activity derived from usage.
 */
export function buildChatSessionDiagram(
  messages: ReadonlyArray<ChatDiagramMessage>
): SessionDiagramModel {
  const turns: DiagramTurn[] = [];
  let current: DiagramTurn | null = null;
  let currentUserTimestamp: string | null = null;

  for (const message of messages) {
    if (message.role === "user" || current === null) {
      current = {
        id: `turn-${turns.length + 1}`,
        number: turns.length + 1,
        timestamp: message.timestamp ?? null,
        nodes: [],
      };
      currentUserTimestamp = null;
      turns.push(current);
    }
    const id = `${current.id}-n${current.nodes.length}`;
    if (message.role === "user") {
      currentUserTimestamp = message.timestamp ?? null;
      current.nodes.push({
        kind: "user_message",
        lane: "user",
        id,
        timestamp: message.timestamp ?? null,
        chars: message.content.length,
        phiExcerpt: phiExcerpt(message.content),
      });
    } else {
      const usage = message.usage ?? null;
      if (
        usage &&
        (usage.cache_read_input_tokens > 0 || usage.cache_write_input_tokens > 0)
      ) {
        current.nodes.push({
          kind: "cache_activity",
          lane: "state",
          id,
          timestamp: message.timestamp ?? null,
          readTokens: usage.cache_read_input_tokens,
          writeTokens: usage.cache_write_input_tokens,
        });
      }
      current.nodes.push({
        kind: "assistant_message",
        lane: "llm",
        id: `${current.id}-n${current.nodes.length}`,
        timestamp: message.timestamp ?? null,
        chars: message.content.length,
        phiExcerpt: phiExcerpt(message.content),
        tally: usage
          ? buildTally(usage, {
              usageComplete: true,
              converseCalls: null,
              toolUses: null,
              stopReason: message.stopReason ?? null,
              durationMs: durationBetween(
                currentUserTimestamp,
                message.timestamp ?? null
              ),
            })
          : null,
      });
    }
  }

  return { surface: "chat", turns };
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

function buildTally(
  usage: TurnUsage,
  extra: Pick<
    DiagramTurnTally,
    "usageComplete" | "converseCalls" | "toolUses" | "stopReason" | "durationMs"
  >
): DiagramTurnTally {
  return {
    modelId: usage.model_id,
    freshInputTokens: usage.input_tokens,
    cacheReadTokens: usage.cache_read_input_tokens,
    cacheWriteTokens: usage.cache_write_input_tokens,
    outputTokens: usage.output_tokens,
    contextTokens:
      usage.input_tokens +
      usage.cache_read_input_tokens +
      usage.cache_write_input_tokens,
    costUsd: usage.cost_usd,
    // Mirrors `accumulateUsage`: pricing_version 0 means the cost is
    // unknown, not zero.
    costKnown: usage.pricing_version !== 0 && Number.isFinite(usage.cost_usd),
    ...extra,
  };
}

/**
 * Collapse whitespace and truncate to the PHI excerpt budget. Returns
 * `null` for effectively-empty text so the renderer can skip the line.
 */
export function phiExcerpt(text: string): string | null {
  const collapsed = text.replace(/\s+/g, " ").trim();
  if (collapsed.length === 0) return null;
  if (collapsed.length <= PHI_EXCERPT_MAX_CHARS) return collapsed;
  return `${collapsed.slice(0, PHI_EXCERPT_MAX_CHARS).trimEnd()}…`;
}

/** Milliseconds between two ISO timestamps; `null` unless both parse in order. */
export function durationBetween(
  startIso: string | null | undefined,
  endIso: string | null | undefined
): number | null {
  if (!startIso || !endIso) return null;
  const start = Date.parse(startIso);
  const end = Date.parse(endIso);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) {
    return null;
  }
  return end - start;
}

/**
 * Format a character count for display, mirroring `formatTokens`'
 * grouping rules ("532 chars", "12.3k chars").
 */
export function formatChars(n: number | null | undefined): string {
  if (n == null || !Number.isFinite(n)) return "—";
  if (n < 10_000) return `${n.toLocaleString()} chars`;
  return `${(n / 1000).toFixed(1)}k chars`;
}

/** Format a duration in milliseconds ("2.4s", "1m 12s"). */
export function formatDurationMs(ms: number): string {
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  let minutes = Math.floor(ms / 60_000);
  let seconds = Math.round((ms % 60_000) / 1000);
  if (seconds === 60) {
    minutes += 1;
    seconds = 0;
  }
  return `${minutes}m ${seconds}s`;
}
