import type { ReactNode } from "react";
import { formatCost, formatTokens, type SessionUsage } from "../lib/cost";
import type { CostLedger } from "../lib/costLedger";
import CostExplanation from "./CostExplanation";

/**
 * The single home for session cost and prompt-cache information. Chat and
 * Writing both use this panel so their default conversation views stay quiet.
 */
export default function SessionUsagePanel({
  session,
  ledger,
  showTurnCosts,
  onShowTurnCostsChange,
  turnCostsLabel = "conversation",
  actions,
}: {
  session: SessionUsage;
  ledger: CostLedger;
  showTurnCosts: boolean;
  onShowTurnCostsChange: (show: boolean) => void;
  turnCostsLabel?: string;
  actions?: ReactNode;
}) {
  const totalTurns = ledger.entries.length + ledger.unmeteredTurns;
  const cacheReadCostUnknown = ledger.entries.some(
    (entry) =>
      entry.components === null && entry.usage.cache_read_input_tokens > 0
  );
  const cacheWriteCostUnknown = ledger.entries.some(
    (entry) =>
      entry.components === null && entry.usage.cache_write_input_tokens > 0
  );
  const recordedSpend =
    session.turnCount === 0
      ? "Not recorded"
      : session.unknownCostTurns > 0 || ledger.unmeteredTurns > 0
        ? `${formatCost(session.totalUsd)} recorded`
        : formatCost(session.totalUsd);

  return (
    <div
      data-testid="session-usage-panel"
      className="h-full overflow-y-auto bg-gray-50 px-5 py-5 select-text"
    >
      <div className="mx-auto max-w-3xl space-y-5">
        <div className="flex flex-wrap items-start justify-between gap-x-5 gap-y-3">
          <div className="min-w-0">
            <h3 className="text-sm font-semibold text-gray-900">
              Session cost &amp; cache
            </h3>
            <p className="mt-0.5 text-xs text-gray-500">
              Spend, cache writes, and reuse for this session only.
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-4">
            {actions}
            <label className="inline-flex cursor-pointer items-center gap-2 text-xs font-medium text-gray-700">
              <input
                type="checkbox"
                aria-label="Show turn costs"
                checked={showTurnCosts}
                disabled={totalTurns === 0}
                onChange={(event) =>
                  onShowTurnCostsChange(event.currentTarget.checked)
                }
                className="h-3.5 w-3.5 rounded border-gray-300 text-blue-600 focus:ring-blue-500 disabled:opacity-50"
              />
              Show turn costs in {turnCostsLabel}
            </label>
          </div>
        </div>

        {totalTurns === 0 ? (
          <div className="rounded-lg border border-dashed border-gray-300 bg-white px-5 py-8 text-center">
            <p className="text-sm font-medium text-gray-700">No usage yet</p>
            <p className="mt-1 text-xs text-gray-500">
              Costs and prompt-cache activity appear after the first turn.
            </p>
          </div>
        ) : (
          <>
            <div className="grid grid-cols-[repeat(auto-fit,minmax(9rem,1fr))] gap-2">
              <UsageMetric
                label="Session spend"
                value={recordedSpend}
                detail={`${totalTurns} turn${totalTurns === 1 ? "" : "s"}`}
              />
              <UsageMetric
                label="Fresh tokens"
                value={formatTokens(session.totalInputTokens)}
                detail={`${formatTokens(session.totalOutputTokens)} output`}
              />
              <UsageMetric
                label="Cache reads"
                value={formatTokens(session.totalCacheReadTokens)}
                detail={
                  cacheReadCostUnknown
                    ? "Read fees unavailable"
                    : `${formatCost(ledger.totals.cacheReadUsd)} read fees`
                }
                accent="emerald"
              />
              <UsageMetric
                label="Cache writes"
                value={formatTokens(session.totalCacheWriteTokens)}
                detail={
                  cacheWriteCostUnknown
                    ? "Write fees unavailable"
                    : `${formatCost(ledger.totals.cacheWriteUsd)} write fees`
                }
                accent="amber"
              />
            </div>

            <CostExplanation ledger={ledger} />
          </>
        )}
      </div>
    </div>
  );
}

function UsageMetric({
  label,
  value,
  detail,
  accent = "gray",
}: {
  label: string;
  value: string;
  detail: string;
  accent?: "gray" | "emerald" | "amber";
}) {
  const valueClass =
    accent === "emerald"
      ? "text-emerald-700"
      : accent === "amber"
        ? "text-amber-700"
        : "text-gray-900";
  return (
    <div className="min-w-0 rounded-lg border border-gray-200 bg-white px-3 py-3">
      <p className="text-[10px] font-semibold uppercase tracking-wide text-gray-400">
        {label}
      </p>
      <p className={`mt-1 truncate text-sm font-semibold ${valueClass}`} title={value}>
        {value}
      </p>
      <p className="mt-0.5 truncate text-[10px] text-gray-500" title={detail}>
        {detail}
      </p>
    </div>
  );
}
