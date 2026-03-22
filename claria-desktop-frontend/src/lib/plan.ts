import type { PlanEntry } from "./tauri";

/** Check whether a plan has any actionable entries. */
export function hasChanges(entries: PlanEntry[] | null): boolean {
  if (!entries) return false;
  return entries.some((e) => e.action !== "ok");
}
