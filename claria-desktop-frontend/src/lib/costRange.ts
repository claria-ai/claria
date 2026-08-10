// Date-range arithmetic for the Cost Explorer.
//
// Separate from the page component because a `.tsx` file that exports
// anything other than components breaks fast refresh, and these two are worth
// testing on their own — the granularity thresholds are the difference
// between a readable chart and several thousand bars.

import type { CostGranularity } from "./tauri";

/** Format a date as the YYYY-MM-DD string the Cost Explorer API expects. */
export function fmtDate(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** The date `n` days before today. */
export function daysAgo(n: number): Date {
  const d = new Date();
  d.setDate(d.getDate() - n);
  return d;
}

/** The first of the month `n` months before this one. */
export function monthsAgo(n: number): Date {
  const d = new Date();
  d.setMonth(d.getMonth() - n);
  d.setDate(1);
  return d;
}

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
