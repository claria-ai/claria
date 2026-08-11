// Quiet resume context for persisted chats. Session dollars live in the
// dedicated costs-and-cache tab rather than competing with the conversation.

import type { ChatHistorySummary } from "../lib/cost";
import { humanRelative } from "../lib/cost";

export default function ChatHistoryHeader({
  summary,
}: {
  summary: ChatHistorySummary;
}) {
  const { turnCount, legacyTurnCount, lastActivityIso } = summary;

  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-gray-100 bg-blue-50/40 px-6 py-2 text-[11px] text-gray-600">
      <span className="font-medium text-gray-700">Resumed conversation</span>
      <span className="text-gray-300">·</span>
      <span>
        {turnCount} {turnCount === 1 ? "turn" : "turns"}
      </span>
      {legacyTurnCount > 0 && (
        <span className="italic text-gray-400">
          ({legacyTurnCount} before usage tracking)
        </span>
      )}
      {lastActivityIso && (
        <>
          <span className="text-gray-300">·</span>
          <span>Last activity {humanRelative(lastActivityIso)}</span>
        </>
      )}
    </div>
  );
}
