// Header shown when the user resumes a previously-saved chat, summarising
// the chat's lifetime cost, turn count, and last-activity time, with a
// link out to the AWS Cost Explorer for account-level spend.

import type { ChatHistorySummary } from "../lib/cost";
import { formatCost, humanRelative } from "../lib/cost";

export default function ChatHistoryHeader({
  summary,
  onSeeAccountSpend,
}: {
  summary: ChatHistorySummary;
  onSeeAccountSpend?: () => void;
}) {
  const { lifetimeUsd, turnCount, legacyTurnCount, lastActivityIso } = summary;

  return (
    <div className="px-6 py-2 border-b border-gray-100 bg-blue-50/40 text-[11px] text-gray-600 flex flex-wrap items-center gap-2">
      <span className="text-gray-700">
        This chat:{" "}
        {lifetimeUsd != null ? (
          <span className="font-semibold text-gray-900">{formatCost(lifetimeUsd)}</span>
        ) : (
          <span className="italic text-gray-400">cost not recorded</span>
        )}{" "}
        over {turnCount} {turnCount === 1 ? "turn" : "turns"}
      </span>
      {legacyTurnCount > 0 && (
        <span className="italic text-gray-400">
          ({legacyTurnCount} {legacyTurnCount === 1 ? "turn pre-dates" : "turns pre-date"} cost
          tracking)
        </span>
      )}
      {lastActivityIso && (
        <>
          <span className="text-gray-300">·</span>
          <span>Last activity: {humanRelative(lastActivityIso)}</span>
        </>
      )}
      {onSeeAccountSpend && (
        <button
          onClick={onSeeAccountSpend}
          className="ml-auto text-blue-600 hover:underline"
        >
          See account spend →
        </button>
      )}
    </div>
  );
}
