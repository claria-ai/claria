import type { PlanEntry } from "./tauri";

/** Check whether a plan has any actionable entries. */
export function hasChanges(entries: PlanEntry[] | null): boolean {
  if (!entries) return false;
  return entries.some((e) => e.action !== "ok");
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
