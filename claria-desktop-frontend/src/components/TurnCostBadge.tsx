// Per-turn cost badge shown under each assistant message bubble.
//
// Surface contract (per #14 / #13):
//   - usage = undefined → render nothing (loading state)
//   - usage = null      → italic grey "cost not recorded" (legacy turn)
//   - cost_usd is NaN / pricing_version === 0 → token counts only, no dollar
//     figure (unknown model pricing — omit rather than show a wrong $0.00)
//   - cost_usd === 0    → "$0.00" (zero is a real number, not missing)
//   - else              → "$0.012 · 1,080 tok cached · 160 tok new"
//
// Hover tooltip shows the full token / model / pricing-rev breakdown.

import type { TurnUsage } from "../lib/tauri";
import { cacheHitPct, formatCost, formatTokens, prettyModel } from "../lib/cost";

export default function TurnCostBadge({
  usage,
}: {
  usage: TurnUsage | null | undefined;
}) {
  if (usage === undefined) return null;
  if (usage === null) return <LegacyTurnLabel />;
  // pricing_version === 0 means lookup failed; cost_usd is 0 in that case.
  const hasCost = usage.pricing_version !== 0 && Number.isFinite(usage.cost_usd);

  const cached = usage.cache_read_input_tokens;
  const newIn = usage.input_tokens;
  const tooltip = buildTooltip(usage, hasCost);

  return (
    <div className="mt-1 flex flex-wrap items-center gap-1.5 text-[10px] text-gray-400 group relative">
      {hasCost && (
        <>
          <span className="font-medium text-gray-500">{formatCost(usage.cost_usd)}</span>
          <span>·</span>
        </>
      )}
      {cached > 0 && (
        <>
          <span className="text-emerald-600">{formatTokens(cached)} cached</span>
          <span>·</span>
        </>
      )}
      <span>{formatTokens(newIn)} new</span>
      <span
        className="invisible group-hover:visible absolute bottom-full left-0 mb-1 z-10 px-2 py-1.5 text-[11px] font-normal text-gray-100 bg-gray-800 rounded whitespace-pre max-w-xs pointer-events-none"
        title={tooltip}
      >
        {tooltip}
      </span>
    </div>
  );
}

function buildTooltip(usage: TurnUsage, hasCost: boolean): string {
  const lines: string[] = [];
  if (hasCost) {
    lines.push(`This turn cost ${formatCost(usage.cost_usd)}`);
  }
  lines.push(`  Input:  ${formatTokens(usage.input_tokens)}`);
  if (usage.cache_read_input_tokens > 0) {
    lines.push(`  Cached: ${formatTokens(usage.cache_read_input_tokens)} (${cacheHitPct(usage)}% hit)`);
  }
  if (usage.cache_write_input_tokens > 0) {
    lines.push(`  Wrote:  ${formatTokens(usage.cache_write_input_tokens)} to cache`);
  }
  lines.push(`  Output: ${formatTokens(usage.output_tokens)}`);
  lines.push(
    hasCost
      ? `Model: ${prettyModel(usage.model_id)} · pricing rev ${usage.pricing_version}`
      : `Model: ${prettyModel(usage.model_id)} · no pricing entry`,
  );
  return lines.join("\n");
}

function LegacyTurnLabel() {
  return (
    <div className="mt-1 text-[10px] italic text-gray-300">cost not recorded</div>
  );
}
