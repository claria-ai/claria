import { useState, type ReactNode } from "react";
import { formatCost, formatTokens, prettyModel } from "../lib/cost";
import {
  formatChars,
  formatDurationMs,
  type DiagramNode,
  type DiagramTurn,
  type DiagramTurnTally,
  type SessionDiagramModel,
} from "../lib/sessionDiagram";

/**
 * Three-lane session flow diagram: user actions, model responses, and
 * Claria state changes, one node sequence per turn, time flowing down.
 *
 * PHI posture: the default view renders only tool names, counts, sizes,
 * token tallies, costs, statuses, model IDs, and timestamps. The
 * "Include PHI" toggle is plain component state — never persisted, so it
 * resets on every mount — and while it is off the PHI strings carried by
 * the model are not rendered into the DOM at all.
 */
export default function SessionFlowDiagram({
  model,
}: {
  model: SessionDiagramModel;
}) {
  const [includePhi, setIncludePhi] = useState(false);
  const hasTurns = model.turns.length > 0;

  return (
    <section data-testid="session-flow-diagram" className="space-y-3">
      <div className="flex flex-wrap items-start justify-between gap-x-5 gap-y-2">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold text-gray-900">Session flow</h3>
          <p className="mt-0.5 text-xs text-gray-500">
            Each turn, from your message through the model&apos;s work to
            Claria&apos;s state.
          </p>
          <p className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-gray-500">
            <span className="inline-flex items-center gap-1">
              <span
                aria-hidden="true"
                className="h-1.5 w-1.5 rounded-full bg-blue-500"
              />
              Fresh input
            </span>
            <span className="inline-flex items-center gap-1">
              <span
                aria-hidden="true"
                className="h-1.5 w-1.5 rounded-full bg-emerald-500"
              />
              Cache read
            </span>
            <span className="inline-flex items-center gap-1">
              <span
                aria-hidden="true"
                className="h-1.5 w-1.5 rounded-full bg-amber-500"
              />
              Cache write
            </span>
          </p>
        </div>
        <label className="inline-flex cursor-pointer items-center gap-2 text-xs font-medium text-gray-700">
          <input
            type="checkbox"
            aria-label="Include PHI"
            checked={includePhi}
            disabled={!hasTurns}
            onChange={(event) => setIncludePhi(event.currentTarget.checked)}
            className="h-3.5 w-3.5 rounded border-gray-300 text-blue-600 focus:ring-blue-500 disabled:opacity-50"
          />
          Include PHI
        </label>
      </div>

      {!hasTurns ? (
        <div className="rounded-lg border border-dashed border-gray-300 bg-white px-5 py-6 text-center">
          <p className="text-sm font-medium text-gray-700">No turns yet</p>
          <p className="mt-1 text-xs text-gray-500">
            Messages, model responses, and tool runs appear here as a
            three-lane timeline after the first turn.
          </p>
        </div>
      ) : (
        <div className="rounded-lg border border-gray-200 bg-white">
          <div className="max-h-[32rem] overflow-y-auto px-3 pb-3">
            <div className="sticky top-0 z-10 grid grid-cols-3 gap-x-1.5 border-b border-gray-200 bg-white pb-1.5 pt-3">
              <p className="text-[10px] font-semibold uppercase tracking-wide text-gray-500">
                You
              </p>
              <p className="text-[10px] font-semibold uppercase tracking-wide text-gray-500">
                Model
              </p>
              <p className="text-[10px] font-semibold uppercase tracking-wide text-gray-500">
                Claria state
              </p>
            </div>
            {model.turns.map((turn) => (
              <TurnGroup
                key={turn.id}
                turn={turn}
                surface={model.surface}
                includePhi={includePhi}
              />
            ))}
          </div>
        </div>
      )}
      {includePhi && hasTurns && (
        <p className="text-[10px] text-gray-500">
          Excerpts are truncated. Open the conversation or record previews
          for full content.
        </p>
      )}
    </section>
  );
}

function TurnGroup({
  turn,
  surface,
  includePhi,
}: {
  turn: DiagramTurn;
  surface: SessionDiagramModel["surface"];
  includePhi: boolean;
}) {
  return (
    <div className="space-y-1.5 pt-2">
      <div className="flex items-center gap-2">
        <span className="text-[10px] font-semibold uppercase tracking-wide text-gray-500">
          Turn {turn.number}
        </span>
        {turn.timestamp && (
          <span className="text-[10px] text-gray-500">
            {formatClock(turn.timestamp)}
          </span>
        )}
        <span aria-hidden="true" className="h-px flex-1 bg-gray-200" />
      </div>
      {turn.nodes.map((node) => (
        <NodeRow
          key={node.id}
          node={node}
          surface={surface}
          includePhi={includePhi}
        />
      ))}
    </div>
  );
}

/**
 * One diagram row: the node chip in its lane column, with a connector
 * glyph in the adjacent column showing which lanes the step flows
 * between (user→model, model↔state, model→user).
 */
function NodeRow({
  node,
  surface,
  includePhi,
}: {
  node: DiagramNode;
  surface: SessionDiagramModel["surface"];
  includePhi: boolean;
}) {
  const chip = <NodeChip node={node} surface={surface} includePhi={includePhi} />;
  if (node.lane === "user") {
    return (
      <div className="grid grid-cols-3 items-center gap-x-1.5">
        <div className="min-w-0">{chip}</div>
        <div aria-hidden="true" className="text-xs text-gray-400">
          →
        </div>
        <div />
      </div>
    );
  }
  if (node.lane === "llm") {
    return (
      <div className="grid grid-cols-3 items-center gap-x-1.5">
        <div aria-hidden="true" className="text-right text-xs text-gray-400">
          ←
        </div>
        <div className="min-w-0">{chip}</div>
        <div />
      </div>
    );
  }
  return (
    <div className="grid grid-cols-3 items-center gap-x-1.5">
      <div />
      <div aria-hidden="true" className="text-right text-xs text-gray-400">
        {node.kind === "context_files" ? "←" : "⇄"}
      </div>
      <div className="min-w-0">{chip}</div>
    </div>
  );
}

function NodeChip({
  node,
  surface,
  includePhi,
}: {
  node: DiagramNode;
  surface: SessionDiagramModel["surface"];
  includePhi: boolean;
}) {
  switch (node.kind) {
    case "user_message":
      return (
        <div className="w-full rounded-md border border-blue-200 bg-blue-50 px-2 py-1.5">
          <ChipHeader
            label={surface === "writer" ? "Instruction" : "Message"}
            labelClass="text-blue-900"
            timestamp={node.timestamp}
          />
          <p className="text-[10px] text-blue-800">{formatChars(node.chars)}</p>
          {includePhi && node.phiExcerpt && (
            <p className="mt-0.5 break-words text-[10px] italic text-blue-900">
              “{node.phiExcerpt}”
            </p>
          )}
        </div>
      );
    case "proposal_decision": {
      const accepted = node.decision === "accepted";
      return (
        <div
          className={
            accepted
              ? "w-full rounded-md border border-emerald-200 bg-emerald-50 px-2 py-1.5"
              : "w-full rounded-md border border-red-200 bg-red-50 px-2 py-1.5"
          }
        >
          <ChipHeader
            label={accepted ? "Accepted proposal" : "Rejected proposal"}
            labelClass={accepted ? "text-emerald-800" : "text-red-800"}
            timestamp={node.timestamp}
          />
          {accepted && (
            <p className="text-[10px] text-emerald-700">
              Saved revision r{node.resultingRevision}
            </p>
          )}
        </div>
      );
    }
    case "assistant_message":
      return (
        <div className="w-full rounded-md border border-gray-200 bg-white px-2 py-1.5">
          <ChipHeader
            label="Reply"
            labelClass="text-gray-800"
            timestamp={node.timestamp}
          />
          <p className="text-[10px] text-gray-600">{formatChars(node.chars)}</p>
          {includePhi && node.phiExcerpt && (
            <p className="mt-0.5 break-words text-[10px] italic text-gray-700">
              “{node.phiExcerpt}”
            </p>
          )}
          {node.tally && <TallyBlock tally={node.tally} />}
        </div>
      );
    case "model_usage":
      return (
        <div className="w-full rounded-md border border-gray-200 bg-white px-2 py-1.5">
          <ChipHeader
            label="Model usage"
            labelClass="text-gray-800"
            timestamp={node.timestamp}
          />
          <TallyBlock tally={node.tally} />
        </div>
      );
    case "tool_activity": {
      const chipClass =
        node.status === "failed"
          ? "w-full rounded-md border border-red-200 bg-red-50 px-2 py-1.5"
          : node.status === "succeeded"
            ? "w-full rounded-md border border-emerald-200 bg-emerald-50 px-2 py-1.5"
            : "w-full rounded-md border border-gray-200 bg-gray-50 px-2 py-1.5";
      const textClass =
        node.status === "failed"
          ? "text-red-700"
          : node.status === "succeeded"
            ? "text-emerald-700"
            : "text-gray-600";
      const labelClass =
        node.status === "failed"
          ? "text-red-800"
          : node.status === "succeeded"
            ? "text-emerald-800"
            : "text-gray-700";
      return (
        <div className={chipClass}>
          <ChipHeader
            label={<code className="font-mono">{node.toolName}</code>}
            labelClass={labelClass}
            timestamp={node.timestamp}
          />
          <p className={`text-[10px] capitalize ${textClass}`}>{node.status}</p>
          <p className={`text-[10px] ${textClass}`}>
            {node.resultChars === null
              ? `call ${formatChars(node.invocationChars)} · no result`
              : `call ${formatChars(node.invocationChars)} · result ${formatChars(node.resultChars)}`}
          </p>
          {includePhi && node.phiSummary && (
            <p className={`mt-0.5 break-words text-[10px] italic ${textClass}`}>
              {node.phiSummary}
            </p>
          )}
        </div>
      );
    }
    case "context_files":
      return (
        <div className="w-full rounded-md border border-gray-200 bg-gray-50 px-2 py-1.5">
          <ChipHeader
            label="Context records"
            labelClass="text-gray-700"
            timestamp={node.timestamp}
          />
          <p className="text-[10px] text-gray-600">
            {node.unavailableCount > 0
              ? `${node.fileCount} file${node.fileCount === 1 ? "" : "s"} (${node.unavailableCount} unavailable)`
              : `${node.fileCount} file${node.fileCount === 1 ? "" : "s"}`}
          </p>
          {includePhi && node.phiFilenames.length > 0 && (
            <p className="mt-0.5 break-words text-[10px] italic text-gray-700">
              {node.phiFilenames.join(", ")}
            </p>
          )}
        </div>
      );
    case "cache_activity":
      return (
        <div className="w-full rounded-md border border-gray-200 bg-gray-50 px-2 py-1.5">
          <ChipHeader
            label="Prompt cache"
            labelClass="text-gray-700"
            timestamp={node.timestamp}
          />
          {node.writeTokens > 0 && (
            <p className="text-[10px] text-amber-700">
              wrote {formatTokens(node.writeTokens)}
            </p>
          )}
          {node.readTokens > 0 && (
            <p className="text-[10px] text-emerald-700">
              read {formatTokens(node.readTokens)}
            </p>
          )}
        </div>
      );
  }
}

function ChipHeader({
  label,
  labelClass,
  timestamp,
}: {
  label: ReactNode;
  labelClass: string;
  timestamp: string | null;
}) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <span className={`min-w-0 truncate text-[10px] font-semibold ${labelClass}`}>
        {label}
      </span>
      {timestamp && (
        <span className="shrink-0 text-[10px] text-gray-500">
          {formatClock(timestamp)}
        </span>
      )}
    </div>
  );
}

/** Per-turn usage tallies under the turn's LLM-lane chip. */
function TallyBlock({ tally }: { tally: DiagramTurnTally }) {
  const durationLabel =
    tally.durationMs !== null ? formatDurationMs(tally.durationMs) : null;
  return (
    <div className="mt-1 space-y-0.5">
      <ContextBar tally={tally} />
      <p className="text-[10px] text-gray-600">
        {`${formatTokens(tally.contextTokens)} context · ${formatTokens(tally.outputTokens)} output`}
      </p>
      <p className="text-[10px] text-gray-600">
        {`${tally.costKnown ? formatCost(tally.costUsd) : "cost unavailable"} · ${prettyModel(tally.modelId)}`}
      </p>
      {tally.converseCalls !== null && tally.toolUses !== null && (
        <p className="text-[10px] text-gray-600">
          {`${tally.converseCalls} model call${tally.converseCalls === 1 ? "" : "s"} · ${tally.toolUses} tool use${tally.toolUses === 1 ? "" : "s"}`}
        </p>
      )}
      {durationLabel && (
        <p className="text-[10px] text-gray-600">{durationLabel}</p>
      )}
      {tally.stopReason === "max_tokens" ? (
        <p className="text-[10px] font-semibold text-red-700">
          Stopped early: hit max tokens
        </p>
      ) : tally.stopReason && tally.stopReason !== "end_turn" ? (
        <p className="text-[10px] text-gray-600">Stop: {tally.stopReason}</p>
      ) : null}
      {!tally.usageComplete && (
        <p className="text-[10px] font-semibold text-amber-700">
          Partial usage capture
        </p>
      )}
    </div>
  );
}

/**
 * Stacked per-turn context bar: fresh input (blue), cache read (emerald),
 * cache write (amber), as shares of the context the model processed.
 * Absolute counts only — no context-window ceiling is drawn here; that
 * constant lives in the backend capability table.
 */
function ContextBar({ tally }: { tally: DiagramTurnTally }) {
  if (tally.contextTokens <= 0) return null;
  const pct = (n: number) => `${(n / tally.contextTokens) * 100}%`;
  const title = `${formatTokens(tally.freshInputTokens)} fresh · ${formatTokens(tally.cacheReadTokens)} cache read · ${formatTokens(tally.cacheWriteTokens)} cache write`;
  return (
    <div
      role="img"
      aria-label={`Context composition: ${title}`}
      title={title}
      className="flex h-1.5 w-full overflow-hidden rounded-full bg-gray-100"
    >
      <span
        className="h-full bg-blue-500"
        style={{ width: pct(tally.freshInputTokens) }}
      />
      <span
        className="h-full bg-emerald-500"
        style={{ width: pct(tally.cacheReadTokens) }}
      />
      <span
        className="h-full bg-amber-500"
        style={{ width: pct(tally.cacheWriteTokens) }}
      />
    </div>
  );
}

/** Local wall-clock time for node/turn headers ("14:32"). */
function formatClock(iso: string): string {
  const t = new Date(iso);
  if (!Number.isFinite(t.getTime())) return "";
  return t.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
}
