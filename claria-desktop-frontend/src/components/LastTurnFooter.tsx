// A quiet ambient line above the composer summarising the last turn.
// Renders only when the most recent assistant turn carries a usage block
// — silent for legacy/missing turns to avoid noisy "$0.00" labels.
// Dollar figures are omitted when the model has no pricing entry.

import type { TurnUsage } from "../lib/tauri";
import { cacheHitPct, formatCost, formatTokens } from "../lib/cost";

export default function LastTurnFooter({
  usage,
  sessionTotalUsd,
}: {
  usage: TurnUsage | null | undefined;
  /** Running session total, or `null` when it can't be computed (a turn
   * without a pricing entry would understate it). */
  sessionTotalUsd: number | null;
}) {
  if (!usage) return null;
  const costKnown = usage.pricing_version !== 0 && Number.isFinite(usage.cost_usd);
  const cacheHit = cacheHitPct(usage);
  return (
    <div className="px-6 py-1.5 border-t border-gray-100 bg-gray-50/50 text-[10px] text-gray-500 flex flex-wrap items-center gap-2">
      <span className="text-gray-600">Last turn:</span>
      {costKnown && (
        <>
          <span className="font-medium text-gray-700">{formatCost(usage.cost_usd)}</span>
          <span className="text-gray-300">·</span>
        </>
      )}
      <span>
        {formatTokens(usage.input_tokens)} in / {formatTokens(usage.output_tokens)} out
      </span>
      {cacheHit > 0 && (
        <>
          <span className="text-gray-300">·</span>
          <span className="text-emerald-600">cache hit {cacheHit}%</span>
        </>
      )}
      {sessionTotalUsd != null && (
        <span className="ml-auto text-gray-400">
          Session total: {formatCost(sessionTotalUsd)}
        </span>
      )}
    </div>
  );
}
