// Always-visible session cost banner mounted at the top of every chat,
// directly under the model selector.
//
// Acts as a fuel gauge: text totals on the left, a progress bar toward
// the hard cap on the right. Tone shifts from gray → amber → red as
// spend approaches the soft / hard cap thresholds.

import type { SessionUsage } from "../lib/cost";
import { formatCost, formatTokens } from "../lib/cost";

export interface SpendCaps {
  softUsd: number;
  hardUsd: number;
}

export default function SessionTotalBanner({
  session,
  caps,
}: {
  session: SessionUsage;
  caps: SpendCaps;
}) {
  const pct = caps.hardUsd > 0 ? Math.min(100, (session.totalUsd / caps.hardUsd) * 100) : 0;
  const tone =
    session.totalUsd >= caps.hardUsd
      ? "red"
      : session.totalUsd >= caps.softUsd
        ? "amber"
        : "gray";

  const textColor =
    tone === "red" ? "text-red-700" : tone === "amber" ? "text-amber-700" : "text-gray-600";
  const barColor =
    tone === "red" ? "bg-red-500" : tone === "amber" ? "bg-amber-500" : "bg-blue-400";
  const bgColor =
    tone === "red" ? "bg-red-50" : tone === "amber" ? "bg-amber-50" : "bg-gray-50";
  const borderColor =
    tone === "red"
      ? "border-red-200"
      : tone === "amber"
        ? "border-amber-200"
        : "border-gray-100";

  return (
    <div
      className={`flex flex-wrap items-center gap-2 px-6 py-1.5 text-[11px] border-b ${bgColor} ${borderColor}`}
    >
      <span className={`font-semibold ${textColor}`}>
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
      <div className="flex items-center gap-2 ml-auto">
        <div className="w-24 h-1.5 rounded-full bg-gray-200 overflow-hidden">
          <div
            className={`h-full ${barColor} transition-all`}
            style={{ width: `${pct}%` }}
          />
        </div>
        <span className="text-gray-400 text-[10px] tabular-nums">
          / {formatCost(caps.hardUsd)}
        </span>
      </div>
    </div>
  );
}
