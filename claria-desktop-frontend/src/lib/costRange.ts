// Date-range arithmetic for the Cost Explorer.
//
// Separate from the page component because a `.tsx` file that exports
// anything other than components breaks fast refresh, and these two are worth
// testing on their own — the granularity thresholds are the difference
// between a readable chart and several thousand bars.

import type { CostGranularity } from "./tauri";

/** Compute the number of days between two date strings. */
export function daysBetween(a: string, b: string): number {
  const da = new Date(a);
  const db = new Date(b);
  return Math.round(Math.abs(db.getTime() - da.getTime()) / 86_400_000);
}

/**
 * Pick default granularity for a date range.
 *
 * Hourly up to a fortnight, daily up to a quarter, monthly beyond. Cost
 * Explorer only retains hourly data when the account has opted in, so
 * `hourlyAvailable` demotes the short-range case to daily.
 */
export function defaultGranularity(
  startDate: string,
  endDate: string,
  hourlyAvailable = true
): CostGranularity {
  const days = daysBetween(startDate, endDate);
  if (days <= 14 && hourlyAvailable) return "hourly";
  if (days <= 90) return "daily";
  return "monthly";
}
