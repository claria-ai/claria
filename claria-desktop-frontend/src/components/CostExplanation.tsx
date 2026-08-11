// Expandable "where did the money go" panel shared by chat and the writer
// timeline. Shows the session ledger rollup (actual vs. would-have-been,
// savings, hit rate) and a compact per-turn list: a stacked bar of the four
// billed components against a dashed ghost bar of the uncached
// counterfactual, annotated for cold starts, timestamp-proven stale windows,
// non-reused prefixes, and cache-investment turns. Purely derived from the
// ledger — no fetching, no persistence — and collapsed by default behind the
// shared accordion chrome.

import { memo, useState } from "react";
import { formatCost } from "../lib/cost";
import {
  cacheTtlLabel,
  positiveLedgerSavings,
  type CostLedger,
  type TurnLedgerEntry,
} from "../lib/costLedger";
import PreferencesSection from "./PreferencesSection";

// Rows rendered until the user asks for everything — keeps the panel cheap
// for multi-hundred-turn sessions (Console's MAX_RENDERED_ENTRIES pattern).
const MAX_RENDERED_TURNS = 50;

export default function CostExplanation({
  ledger,
  className,
  compact = false,
}: {
  ledger: CostLedger | null | undefined;
  /** Outer wrapper classes — the only thing that differs per surface. */
  className?: string;
  compact?: boolean;
}) {
  const [showAll, setShowAll] = useState(false);
  if (!ledger || ledger.entries.length === 0) return null;

  const visible = showAll
    ? ledger.entries
    : ledger.entries.slice(-MAX_RENDERED_TURNS);
  const hiddenCount = ledger.entries.length - visible.length;
  // Scale every bar against the most expensive rendered turn (actual or
  // ghost) so widths are comparable across rows.
  let maxUsd = 0;
  for (const entry of visible) {
    maxUsd = Math.max(maxUsd, entry.actualUsd ?? 0, entry.counterfactualUsd ?? 0);
  }

  return (
    <div className={className}>
      <PreferencesSection
        title="Cost breakdown"
        testId="cost-explanation"
        summary={<SummaryChip ledger={ledger} compact={compact} />}
        className={compact ? "border-0 rounded-none h-full" : undefined}
        summaryClassName={
          compact
            ? "h-full flex items-center justify-between px-3 py-1.5 cursor-pointer list-none [&::-webkit-details-marker]:hidden"
            : undefined
        }
        titleClassName={
          compact ? "text-[11px] font-semibold text-gray-600" : undefined
        }
        contentClassName={
          compact
            ? "border-t border-gray-100 p-3 space-y-2"
            : "border-t border-gray-100 p-4 space-y-3"
        }
      >
        <SessionSummary ledger={ledger} />
        {hiddenCount > 0 && (
          <button
            type="button"
            onClick={() => setShowAll(true)}
            className="text-xs font-medium text-blue-700 hover:text-blue-900"
          >
            Show all {ledger.entries.length} turns ({hiddenCount} hidden)
          </button>
        )}
        <ol className="space-y-2" aria-label="Per-turn cost breakdown">
          {visible.map((entry, i) => (
            <TurnRow
              key={entry.index}
              entry={entry}
              turnNumber={hiddenCount + i + 1}
              maxUsd={maxUsd}
            />
          ))}
        </ol>
        <Legend />
      </PreferencesSection>
    </div>
  );
}

function SummaryChip({
  ledger,
  compact,
}: {
  ledger: CostLedger;
  compact: boolean;
}) {
  const savings = positiveLedgerSavings(ledger);
  return (
    <span
      className={
        compact ? "text-[10px] text-gray-500" : "text-xs text-gray-500"
      }
    >
      {formatCost(ledger.actualUsd)}
      {savings && (
        <span className="text-emerald-600">
          {" "}
          · saved {formatCost(savings.usd)}
        </span>
      )}
    </span>
  );
}

function SessionSummary({ ledger }: { ledger: CostLedger }) {
  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-center gap-2 text-xs text-gray-600">
        <span>
          Actual{" "}
          <span className="font-semibold">{formatCost(ledger.actualUsd)}</span>
        </span>
        <span className="text-gray-300">·</span>
        <span>
          Without caching{" "}
          <span className="font-semibold">
            {formatCost(ledger.counterfactualUsd)}
          </span>
        </span>
        <span className="text-gray-300">·</span>
        {ledger.savingsUsd >= 0 ? (
          <span className="text-emerald-700 font-medium">
            Saved {formatCost(ledger.savingsUsd)} (
            {Math.round(ledger.savingsPct)}%)
          </span>
        ) : (
          <span className="text-blue-700 font-medium">
            {formatCost(-ledger.savingsUsd)} invested in cache so far
          </span>
        )}
        <span className="text-gray-300">·</span>
        <span>
          Cache hits {ledger.hitCount} of {ledger.entries.length} turn
          {ledger.entries.length === 1 ? "" : "s"}
        </span>
        {ledger.missCount > 0 && (
          <span className="text-amber-700">
            {ledger.missCount} stale cache window
            {ledger.missCount === 1 ? "" : "s"}
          </span>
        )}
        {ledger.notReusedCount > 0 && (
          <span className="text-gray-500">
            {ledger.notReusedCount} cache prefix
            {ledger.notReusedCount === 1 ? " was" : "es were"} not reused
          </span>
        )}
      </div>
      {(ledger.unmeteredTurns > 0 || ledger.unpricedTurns > 0) && (
        <p className="text-[11px] text-gray-400">
          {[
            ledger.unmeteredTurns > 0
              ? `${ledger.unmeteredTurns} turn${ledger.unmeteredTurns === 1 ? "" : "s"} without recorded usage`
              : null,
            ledger.unpricedTurns > 0
              ? `${ledger.unpricedTurns} turn${ledger.unpricedTurns === 1 ? "" : "s"} without a pricing entry — totals exclude ${ledger.unpricedTurns === 1 ? "it" : "them"}`
              : null,
          ]
            .filter(Boolean)
            .join("; ")}
        </p>
      )}
    </div>
  );
}

// Memoized on the entry object: the ledger is rebuilt only when usage or
// pricing changes, so rows skip re-rendering on unrelated parent state.
const TurnRow = memo(function TurnRow({
  entry,
  turnNumber,
  maxUsd,
}: {
  entry: TurnLedgerEntry;
  turnNumber: number;
  maxUsd: number;
}) {
  const { components, actualUsd, counterfactualUsd, savingsUsd } = entry;
  if (components === null || actualUsd === null || counterfactualUsd === null) {
    return (
      <li className="text-[11px] text-gray-400">
        <span className="font-medium text-gray-500">Turn {turnNumber}</span>{" "}
        <span className="italic">no pricing entry for this model</span>
      </li>
    );
  }

  const segments = [
    {
      label: "Fresh input",
      usd: components.freshInputUsd,
      barClass: "bg-blue-600",
    },
    {
      label: `Cache write (${cacheTtlLabel(entry.cacheWriteTtl)})`,
      usd: components.cacheWriteUsd,
      barClass: "bg-amber-500",
    },
    {
      label: "Cache read",
      usd: components.cacheReadUsd,
      barClass: "bg-emerald-600",
    },
    { label: "Output", usd: components.outputUsd, barClass: "bg-violet-600" },
  ];
  const breakdown = segments
    .filter((segment) => segment.usd > 0)
    .map((segment) => `${segment.label}: ${formatCost(segment.usd)}`)
    .join(", ");
  const widthPct = (usd: number) =>
    maxUsd > 0 ? `${((usd / maxUsd) * 100).toFixed(2)}%` : "0%";

  return (
    <li className="text-[11px]">
      <div className="flex flex-wrap items-center gap-2 text-gray-500">
        <span className="font-medium text-gray-600">Turn {turnNumber}</span>
        <span className="font-medium text-gray-700">
          {formatCost(actualUsd)}
        </span>
        {entry.outcome.kind === "cold_start" && (
          <span className="text-gray-400">cold start</span>
        )}
        {entry.outcome.kind === "miss" && (
          <span className="text-amber-700">
            {cacheTtlLabel(entry.outcome.expiredTtl)} cache was stale
          </span>
        )}
        {entry.outcome.kind === "not_reused" && (
          <span className="text-gray-400">cache not reused</span>
        )}
        {savingsUsd !== null && savingsUsd > 0 && (
          <span className="text-emerald-600">
            saved {formatCost(savingsUsd)}
          </span>
        )}
        {savingsUsd !== null && savingsUsd < 0 && (
          <span className="text-blue-700">
            invested {formatCost(-savingsUsd)} in cache
          </span>
        )}
      </div>
      <div
        role="img"
        aria-label={`Turn ${turnNumber} cost ${formatCost(actualUsd)}${breakdown ? ` (${breakdown})` : ""}; without caching ${formatCost(counterfactualUsd)}`}
        title={breakdown}
        className="mt-1 flex h-2 w-full overflow-hidden rounded bg-gray-100"
      >
        {segments.map((segment) =>
          segment.usd > 0 ? (
            <span
              key={segment.label}
              className={`${segment.barClass} h-full`}
              style={{ width: widthPct(segment.usd) }}
              title={`${segment.label}: ${formatCost(segment.usd)}`}
            />
          ) : null
        )}
      </div>
      <div
        aria-hidden="true"
        title={`Without caching: ${formatCost(counterfactualUsd)}`}
        className="mt-0.5 h-1.5 rounded border border-dashed border-gray-400"
        style={{ width: widthPct(counterfactualUsd) }}
      />
    </li>
  );
});

function Legend() {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-gray-500">
      <span className="inline-flex items-center gap-1">
        <span aria-hidden="true" className="inline-block h-2 w-3 rounded-sm bg-blue-600" />
        Fresh input
      </span>
      <span className="inline-flex items-center gap-1">
        <span aria-hidden="true" className="inline-block h-2 w-3 rounded-sm bg-amber-500" />
        Cache write
      </span>
      <span className="inline-flex items-center gap-1">
        <span aria-hidden="true" className="inline-block h-2 w-3 rounded-sm bg-emerald-600" />
        Cache read
      </span>
      <span className="inline-flex items-center gap-1">
        <span aria-hidden="true" className="inline-block h-2 w-3 rounded-sm bg-violet-600" />
        Output
      </span>
      <span className="inline-flex items-center gap-1">
        <span
          aria-hidden="true"
          className="inline-block h-1.5 w-3 rounded border border-dashed border-gray-400"
        />
        Without caching
      </span>
    </div>
  );
}
