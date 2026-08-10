import { useState, useEffect, useCallback, useMemo } from "react";
import {
  getCostAndUsage,
  probeCostExplorer,
  enableCostExplorer,
  loadConfig,
  type CostGranularity,
  type CostAndUsageResult,
  type CostTimePeriod,
  type CostResultGroup,
} from "../lib/tauri";
import { costErrorMessage } from "../lib/costErrors";
import {
  daysAgo,
  daysBetween,
  defaultGranularity,
  fmtDate,
  monthsAgo,
} from "../lib/costRange";
import { useAsyncLoad } from "../lib/useAsyncLoad";
import { BackButton, CloseIcon } from "../components/icons";
import Spinner from "../components/Spinner";
import type { Page } from "../App";

// ---------------------------------------------------------------------------
// Service tooltip descriptions
// ---------------------------------------------------------------------------

const SERVICE_TOOLTIPS: Record<string, string> = {
  "Amazon Bedrock":
    "AI model usage — powers the chat assistant and report generation. " +
    "For per-chat detail, see the cost banner inside each chat session.",
  "Amazon Simple Storage Service":
    "File storage — your client records, documents, and backups",
  "AWS CloudTrail":
    "Audit logging — records all account activity for HIPAA compliance",
  "Amazon Transcribe":
    "Audio transcription — converts session recordings to text",
  "AWS Key Management Service":
    "Encryption key management — protects your data at rest",
  "AWS Cost Explorer":
    "Cost Explorer API calls — each cost lookup costs $0.01",
};

// ---------------------------------------------------------------------------
// Distinct colors for stacked service bars
// ---------------------------------------------------------------------------

const BAR_COLORS = [
  "bg-blue-500",
  "bg-emerald-500",
  "bg-amber-500",
  "bg-rose-500",
  "bg-violet-500",
  "bg-cyan-500",
  "bg-orange-500",
  "bg-pink-500",
  "bg-teal-500",
  "bg-indigo-500",
  "bg-lime-500",
  "bg-fuchsia-500",
];

// ---------------------------------------------------------------------------
// Date helpers
// ---------------------------------------------------------------------------

function firstOfMonth(): Date {
  const d = new Date();
  d.setDate(1);
  return d;
}

// ---------------------------------------------------------------------------
// Time-range presets
// ---------------------------------------------------------------------------

interface Preset {
  label: string;
  start: () => Date;
  end: () => Date;
  granularity: CostGranularity;
}

const PRESETS: Preset[] = [
  {
    label: "Last 24h",
    start: () => daysAgo(1),
    end: () => new Date(),
    granularity: "hourly",
  },
  {
    label: "Last 7d",
    start: () => daysAgo(7),
    end: () => new Date(),
    granularity: "hourly",
  },
  {
    label: "Last 14d",
    start: () => daysAgo(14),
    end: () => new Date(),
    granularity: "hourly",
  },
  {
    label: "Last 30d",
    start: () => daysAgo(30),
    end: () => new Date(),
    granularity: "daily",
  },
  {
    label: "Month to date",
    start: firstOfMonth,
    end: () => new Date(),
    granularity: "daily",
  },
  {
    label: "Last 3mo",
    start: () => monthsAgo(3),
    end: () => new Date(),
    granularity: "daily",
  },
  {
    label: "Last 12mo",
    start: () => monthsAgo(12),
    end: () => new Date(),
    granularity: "monthly",
  },
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Format a period label for x-axis display. */
function periodLabel(period: CostTimePeriod, granularity: CostGranularity): string {
  const start = period.start;
  if (granularity === "hourly") {
    // "2026-03-01T00:00:00Z" → "Mar 1 00:00" or just the time if same day
    const d = new Date(start + "T00:00:00");
    if (start.includes("T")) {
      const dt = new Date(start);
      return dt.toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
    }
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }
  if (granularity === "monthly") {
    const d = new Date(start + "T00:00:00");
    return d.toLocaleDateString(undefined, { year: "numeric", month: "short" });
  }
  // daily
  const d = new Date(start + "T00:00:00");
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export default function CostExplorer({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  const configLoad = useAsyncLoad(() => loadConfig(), []);
  const loading = configLoad.loading;
  // Set once the user completes onboarding in this session.
  const [justEnabled, setJustEnabled] = useState(false);
  const costExplorerEnabled =
    justEnabled || (configLoad.data?.cost_explorer_enabled ?? false);
  const hourlyAvailable = configLoad.data?.hourly_cost_data ?? false;

  if (loading) {
    return (
      <div className="max-w-4xl mx-auto p-8">
        <p className="text-gray-500">Loading...</p>
      </div>
    );
  }

  return (
    <div className="max-w-4xl mx-auto p-8 overflow-x-clip">
      {/* Header */}
      <div className="flex items-center gap-3 mb-6">
        <BackButton onClick={() => navigate("provision")} />
        <h2 className="text-2xl font-bold">Cost Explorer</h2>
      </div>

      {costExplorerEnabled ? (
        <CostChart hourlyAvailable={hourlyAvailable} />
      ) : (
        <Onboarding onEnabled={() => setJustEnabled(true)} />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Mode A: Onboarding
// ---------------------------------------------------------------------------

function Onboarding({ onEnabled }: { onEnabled: () => void }) {
  const [probing, setProbing] = useState(false);
  const [probeError, setProbeError] = useState<string | null>(null);

  async function handleVerify() {
    setProbing(true);
    setProbeError(null);
    try {
      await probeCostExplorer();
      await enableCostExplorer();
      onEnabled();
    } catch (e) {
      setProbeError(
        costErrorMessage(e, {
          dataUnavailable:
            "Cost Explorer isn't enabled yet, or data hasn't appeared. " +
            "It can take up to 24 hours after enabling. Please check the AWS Console and try again.",
          accessDenied:
            "Claria doesn't have permission to access Cost Explorer. " +
            "Go to AWS \u2192 Re-scan to update your IAM policy.",
          fallback:
            String(e) ||
            "Couldn't reach AWS. Check your internet connection and try again.",
        })
      );
    } finally {
      setProbing(false);
    }
  }

  return (
    <div className="space-y-6">
      <div className="bg-white border border-gray-200 rounded-lg p-6">
        <h3 className="text-lg font-semibold mb-3">
          View your AWS spending
        </h3>
        <p className="text-sm text-gray-600 mb-4">
          See a breakdown of your AWS costs by service, day, or month — without
          leaving Claria.
        </p>

        <div className="bg-amber-50 border border-amber-200 rounded-lg p-4 mb-4">
          <p className="text-sm text-amber-800">
            <strong>Pricing:</strong> Each time you load cost data, AWS charges
            $0.01 to your account. The AWS Cost Explorer web console is free —
            this is a convenience feature.
          </p>
        </div>

        <h4 className="text-sm font-semibold text-gray-800 mb-2">
          Setup steps
        </h4>
        <ol className="list-decimal list-inside text-sm text-gray-600 space-y-1.5 mb-6">
          <li>Sign in to the AWS Console</li>
          <li>
            Go to <strong>Billing &rarr; Cost Explorer</strong>
          </li>
          <li>
            Click <strong>"Enable Cost Explorer"</strong>
          </li>
          <li>Wait up to 24 hours for data to appear</li>
        </ol>

        <button
          onClick={handleVerify}
          disabled={probing}
          className="px-4 py-2 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 flex items-center gap-2"
        >
          {probing && <Spinner />}
          {probing
            ? "Verifying..."
            : "I've enabled Cost Explorer \u2014 verify"}
        </button>

        {probeError && (
          <div className="bg-red-50 border border-red-200 rounded-lg p-4 mt-4">
            <p className="text-red-800 text-sm">{probeError}</p>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Mode B: Active chart
// ---------------------------------------------------------------------------

function CostChart({ hourlyAvailable }: { hourlyAvailable: boolean }) {
  // The committed date range. Changing it costs $0.01, so only a preset click
  // or the Apply button below writes to it.
  const [startDate, setStartDate] = useState(() => fmtDate(daysAgo(30)));
  const [endDate, setEndDate] = useState(() => fmtDate(new Date()));

  // What the two date inputs currently show. A `type="date"` input fires
  // change events mid-edit — once per field as the user types a year — so
  // these stay local until the user commits them.
  const [draftStart, setDraftStart] = useState(startDate);
  const [draftEnd, setDraftEnd] = useState(endDate);
  const rangeDirty = draftStart !== startDate || draftEnd !== endDate;

  // Controls
  const [granularity, setGranularity] = useState<CostGranularity>("daily");
  const [groupByService, setGroupByService] = useState(true);

  // Data
  const [result, setResult] = useState<CostAndUsageResult | null>(null);
  const [fetching, setFetching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Session call counter
  const [callCount, setCallCount] = useState(0);
  const [bannerDismissed, setBannerDismissed] = useState(false);

  // Active preset tracking
  const [activePreset, setActivePreset] = useState<string | null>("Last 30d");

  // Service filter — click a legend item to isolate one service
  const [serviceFilter, setServiceFilter] = useState<string | null>(null);

  const fetchData = useCallback(
    async (start: string, end: string, gran: CostGranularity, grouped: boolean) => {
      setFetching(true);
      setError(null);
      try {
        const data = await getCostAndUsage(start, end, gran, grouped);
        setResult(data);
        setCallCount((c) => c + 1);
      } catch (e) {
        setError(
          costErrorMessage(e, {
            dataUnavailable:
              "Cost Explorer data is not available. It can take up to 24 hours after enabling.",
            accessDenied:
              "Claria doesn't have permission to view billing data. Go to AWS \u2192 Re-scan to update your IAM policy.",
          })
        );
      } finally {
        setFetching(false);
      }
    },
    []
  );

  // Fetch on mount + when a committed control changes. Every entry here is a
  // single deliberate click, so it maps one-to-one onto a $0.01 charge.
  useEffect(() => {
    fetchData(startDate, endDate, granularity, groupByService);
  }, [startDate, endDate, granularity, groupByService, fetchData]);

  function handlePreset(preset: Preset) {
    const s = fmtDate(preset.start());
    const e = fmtDate(preset.end());
    setStartDate(s);
    setEndDate(e);
    setDraftStart(s);
    setDraftEnd(e);
    const gran = preset.granularity === "hourly" && !hourlyAvailable
      ? "daily"
      : preset.granularity;
    setGranularity(gran);
    setActivePreset(preset.label);
  }

  function handleApplyRange() {
    if (!rangeDirty) return;
    setStartDate(draftStart);
    setEndDate(draftEnd);
    setGranularity(defaultGranularity(draftStart, draftEnd, hourlyAvailable));
    setActivePreset(null);
  }

  function handleGranularity(g: CostGranularity) {
    setGranularity(g);
  }

  // Granularity availability
  const days = daysBetween(startDate, endDate);
  const canHourly = days <= 14;

  // Apply the service filter once per fetch/filter change; every consumer
  // (chart bars, totals, max) reads these precomputed rows.
  const filteredPeriods = useMemo<FilteredPeriod[]>(() => {
    if (!result) return [];
    return result.periods.map((period) => {
      const groups = serviceFilter
        ? period.groups.filter((g) => g.key === serviceFilter)
        : period.groups;
      const total = groups.reduce(
        (sum, g) => sum + parseFloat(g.amount || "0"),
        0
      );
      return { period, groups, total };
    });
  }, [result, serviceFilter]);

  const { maxTotal, allServices } = useMemo(() => {
    let max = 0;
    const svcSet = new Set<string>();
    for (const { period, total } of filteredPeriods) {
      if (total > max) max = total;
      for (const g of period.groups) {
        if (g.key !== "Total") svcSet.add(g.key);
      }
    }
    return { maxTotal: max, allServices: Array.from(svcSet).sort() };
  }, [filteredPeriods]);

  const serviceColors = useMemo(() => {
    const map: Record<string, string> = {};
    allServices.forEach((s, i) => {
      map[s] = BAR_COLORS[i % BAR_COLORS.length];
    });
    return map;
  }, [allServices]);

  // Total cost across all periods (respects service filter)
  const grandTotal = useMemo(
    () => filteredPeriods.reduce((sum, p) => sum + p.total, 0),
    [filteredPeriods]
  );

  return (
    <div className="space-y-4">
      {/* Info line */}
      <p className="text-xs text-gray-400">
        Cost data is delayed ~24 hours. Each data refresh costs $0.01.
      </p>

      {/* Session call count warning banner */}
      {callCount >= 20 && !bannerDismissed && (
        <div className="bg-amber-50 border border-amber-200 rounded-lg p-3 flex items-start gap-3">
          <p className="text-sm text-amber-800 flex-1">
            You've made {callCount} cost lookups this session. Each lookup
            costs $0.01 on your AWS bill. The AWS Cost Explorer web console
            offers the same data for free.
          </p>
          <button
            onClick={() => setBannerDismissed(true)}
            aria-label="Dismiss"
            className="text-amber-500 hover:text-amber-700 shrink-0"
          >
            <CloseIcon className="w-4 h-4" />
          </button>
        </div>
      )}

      {/* Preset buttons */}
      <div className="flex flex-wrap gap-1.5">
        {PRESETS.map((p) => (
          <button
            key={p.label}
            onClick={() => handlePreset(p)}
            disabled={fetching}
            className={`px-2.5 py-1 text-xs rounded-lg transition-colors disabled:opacity-50 ${
              activePreset === p.label
                ? "bg-blue-600 text-white"
                : "bg-gray-100 text-gray-600 hover:bg-gray-200"
            }`}
          >
            {p.label}
          </button>
        ))}
      </div>

      {/* Custom date picker + controls */}
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-1.5">
          <input
            type="date"
            value={draftStart}
            min={fmtDate(monthsAgo(13))}
            max={draftEnd}
            onChange={(e) => setDraftStart(e.target.value)}
            disabled={fetching}
            className="px-2 py-1 text-xs border border-gray-300 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
          />
          <span className="text-xs text-gray-400">&ndash;</span>
          <input
            type="date"
            value={draftEnd}
            min={draftStart}
            max={fmtDate(new Date())}
            onChange={(e) => setDraftEnd(e.target.value)}
            disabled={fetching}
            className="px-2 py-1 text-xs border border-gray-300 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500"
          />
          <button
            onClick={handleApplyRange}
            disabled={fetching || !rangeDirty}
            title={
              rangeDirty
                ? "Load this range — costs $0.01"
                : "This range is already loaded"
            }
            className="px-2.5 py-1 text-xs rounded-lg transition-colors bg-gray-100 text-gray-600 hover:bg-gray-200 disabled:opacity-40 disabled:hover:bg-gray-100"
          >
            Apply
          </button>
        </div>

        {/* Granularity */}
        <div className="flex rounded-lg border border-gray-200 overflow-hidden">
          {(["hourly", "daily", "monthly"] as const)
            .filter((g) => g !== "hourly" || hourlyAvailable)
            .map((g, i) => {
            const disabled = g === "hourly" && !canHourly;
            const active = granularity === g;
            return (
              <button
                key={g}
                onClick={() => handleGranularity(g)}
                disabled={fetching || disabled}
                title={
                  disabled
                    ? "Hourly granularity requires a range of 14 days or less"
                    : undefined
                }
                className={`px-2.5 py-1 text-xs capitalize transition-colors ${
                  active
                    ? "bg-blue-600 text-white"
                    : disabled
                      ? "text-gray-300 cursor-not-allowed"
                      : "text-gray-600 hover:bg-gray-100"
                } ${i > 0 ? "border-l border-gray-200" : ""}`}
              >
                {g}
              </button>
            );
          })}
        </div>

        {/* Group by service */}
        <label className="flex items-center gap-1.5 text-xs text-gray-600">
          <input
            type="checkbox"
            checked={groupByService}
            onChange={(e) => setGroupByService(e.target.checked)}
            disabled={fetching}
            className="rounded border-gray-300"
          />
          By service
        </label>
      </div>

      {/* Loading */}
      {fetching && (
        <div className="flex items-center gap-2 text-gray-500 text-sm py-4">
          <Spinner />
          <span>Loading cost data...</span>
        </div>
      )}

      {/* Error */}
      {error && !fetching && (
        <div className="bg-red-50 border border-red-200 rounded-lg p-4">
          <p className="text-red-800 text-sm">{error}</p>
        </div>
      )}

      {/* Chart */}
      {result && !fetching && !error && (
        <>
          {/* Grand total */}
          <div className="text-sm text-gray-700">
            {serviceFilter ? serviceFilter : "Total"}: <strong>${grandTotal.toFixed(2)}</strong>
          </div>

          {result.periods.length === 0 ? (
            <div className="bg-gray-50 border border-gray-200 rounded-lg p-6 text-center">
              <p className="text-gray-500 text-sm">
                No cost data available for this time period.
              </p>
            </div>
          ) : (
            <div className="bg-white border border-gray-200 rounded-lg p-4">
              <BarChart
                periods={filteredPeriods}
                granularity={granularity}
                maxTotal={maxTotal}
                groupByService={groupByService}
                serviceColors={serviceColors}
                serviceFilter={serviceFilter}
              />

              {/* Legend */}
              {groupByService && allServices.length > 0 && (
                <div className="flex flex-wrap gap-x-4 gap-y-1.5 mt-4 pt-3 border-t border-gray-100">
                  {allServices.map((svc) => (
                    <ServiceLegendItem
                      key={svc}
                      name={svc}
                      colorClass={serviceColors[svc]}
                      active={serviceFilter === null || serviceFilter === svc}
                      onClick={() =>
                        setServiceFilter((f) => (f === svc ? null : svc))
                      }
                    />
                  ))}
                  {serviceFilter && (
                    <button
                      onClick={() => setServiceFilter(null)}
                      className="text-xs text-blue-600 hover:text-blue-800 ml-1"
                    >
                      Clear filter
                    </button>
                  )}
                </div>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Bar chart
// ---------------------------------------------------------------------------

/** One chart bar's precomputed data: the period plus its filtered view. */
interface FilteredPeriod {
  period: CostTimePeriod;
  groups: CostResultGroup[];
  total: number;
}

function BarChart({
  periods,
  granularity,
  maxTotal,
  groupByService,
  serviceColors,
  serviceFilter,
}: {
  periods: FilteredPeriod[];
  granularity: CostGranularity;
  maxTotal: number;
  groupByService: boolean;
  serviceColors: Record<string, string>;
  serviceFilter: string | null;
}) {
  const chartHeight = 200;

  return (
    <div className="flex items-end gap-px" style={{ height: chartHeight }}>
      {periods.map(({ period, groups, total }, i) => {
        const heightPct = maxTotal > 0 ? (total / maxTotal) * 100 : 0;
        const label = periodLabel(period, granularity);

        return (
          <div
            key={i}
            className="flex-1 flex flex-col items-stretch justify-end min-w-0 group relative"
            style={{ height: "100%" }}
          >
            {/* Stacked bar */}
            <div
              className="w-full flex flex-col justify-end overflow-hidden rounded-t"
              style={{ height: `${heightPct}%`, minHeight: total > 0 ? 2 : 0 }}
            >
              {groupByService && groups.length > 0
                ? groups
                    .filter((g) => g.key !== "Total")
                    .map((g) => {
                      const amt = parseFloat(g.amount || "0");
                      const segPct =
                        total > 0 ? (amt / total) * 100 : 0;
                      return (
                        <div
                          key={g.key}
                          className={`w-full ${serviceColors[g.key] ?? "bg-gray-400"}`}
                          style={{ height: `${segPct}%`, minHeight: amt > 0 ? 1 : 0 }}
                        />
                      );
                    })
                : (
                  <div className="w-full bg-blue-500 flex-1 rounded-t" />
                )}
            </div>

            {/* X-axis label */}
            {periods.length <= 40 && (
              <div className="text-center mt-1">
                <span className="text-[9px] text-gray-400 leading-none truncate block">
                  {label}
                </span>
              </div>
            )}

            {/* Tooltip on hover */}
            <Tooltip period={period} total={total} granularity={granularity} serviceFilter={serviceFilter} />
          </div>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Tooltip
// ---------------------------------------------------------------------------

function Tooltip({
  period,
  total,
  granularity,
  serviceFilter,
}: {
  period: CostTimePeriod;
  total: number;
  granularity: CostGranularity;
  serviceFilter: string | null;
}) {
  const groups = serviceFilter
    ? period.groups.filter((g) => g.key === serviceFilter)
    : period.groups;

  return (
    <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 hidden group-hover:block z-10 pointer-events-none">
      <div className="bg-gray-900 text-white text-xs rounded-lg px-3 py-2 shadow-lg whitespace-nowrap">
        <p className="font-medium mb-1">
          {periodLabel(period, granularity)}: ${total.toFixed(2)}
        </p>
        {groups
          .filter((g) => parseFloat(g.amount || "0") > 0)
          .sort(
            (a, b) =>
              parseFloat(b.amount || "0") - parseFloat(a.amount || "0")
          )
          .slice(0, 8)
          .map((g) => (
            <p key={g.key} className="text-gray-300">
              {g.key}: ${parseFloat(g.amount || "0").toFixed(4)}
            </p>
          ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Service legend item with tooltip
// ---------------------------------------------------------------------------

function ServiceLegendItem({
  name,
  colorClass,
  active,
  onClick,
}: {
  name: string;
  colorClass: string;
  active: boolean;
  onClick: () => void;
}) {
  const tooltip = SERVICE_TOOLTIPS[name] ?? "Other AWS service usage in this account";

  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-1.5 group/legend relative transition-opacity ${
        active ? "opacity-100" : "opacity-30"
      }`}
    >
      <div className={`w-2.5 h-2.5 rounded-sm ${colorClass}`} />
      <span className="text-xs text-gray-600 hover:text-gray-900">{name}</span>
      <div className="absolute bottom-full left-0 mb-1 hidden group-hover/legend:block z-10 pointer-events-none">
        <div className="bg-gray-900 text-white text-xs rounded-lg px-2.5 py-1.5 shadow-lg max-w-xs">
          {tooltip}
        </div>
      </div>
    </button>
  );
}
