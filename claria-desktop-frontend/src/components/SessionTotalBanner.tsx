// Always-visible session cost banner mounted at the top of every chat,
// directly under the model selector. Surfaces running spend so the user
// can decide when to stop — no enforcement, no thresholds.

import type { SessionUsage } from "../lib/cost";
import { formatCost, formatTokens } from "../lib/cost";

export default function SessionTotalBanner({
  session,
}: {
  session: SessionUsage;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2 px-6 py-1.5 text-[11px] border-b bg-gray-50 border-gray-100">
      <span className="font-semibold text-gray-600">
        Session: {formatCost(session.totalUsd)}
      </span>
      <span className="text-gray-400">·</span>
      <span className="text-gray-500">{formatTokens(session.totalInputTokens)} in</span>
      <span className="text-gray-400">·</span>
      <span className="text-gray-500">{formatTokens(session.totalOutputTokens)} out</span>
      {session.cacheSavedUsd > 0 && (
        <>
          <span className="text-gray-400">·</span>
          <span className="text-emerald-600 font-medium">
            saved {formatCost(session.cacheSavedUsd)} via cache
          </span>
        </>
      )}
    </div>
  );
}
