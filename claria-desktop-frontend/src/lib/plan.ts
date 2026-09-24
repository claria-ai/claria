import type { PlanEntry } from "./tauri";

/**
 * Check whether a plan has any actionable entries.
 *
 * An unreadable resource is not one. Nothing is known about it, so there is
 * nothing for Apply to do — counting it would offer the operator a button that
 * cannot change anything.
 */
export function hasChanges(entries: PlanEntry[] | null): boolean {
  if (!entries) return false;
  return entries.some((e) => e.action !== "ok" && e.action !== "unknown");
}

/** The resources this scan could not read. */
export function unreadableEntries(entries: PlanEntry[] | null): PlanEntry[] {
  if (!entries) return [];
  return entries.filter((e) => e.action === "unknown");
}

/** Find the entry that needs elevated credentials to create/modify, if any. */
export function findEscalationEntry(entries: PlanEntry[] | null): PlanEntry | null {
  if (!entries) return null;
  return (
    entries.find(
      (e) =>
        e.spec.credential_scope === "elevated" &&
        (e.action === "create" || e.action === "modify")
    ) ?? null
  );
}
