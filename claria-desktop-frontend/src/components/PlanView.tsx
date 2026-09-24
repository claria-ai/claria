import type { ReactNode } from "react";
import type { PlanEntry } from "../lib/tauri";
import type { ApplyItem } from "../lib/provisioner";
import { unreadableEntries } from "../lib/plan";
import PlanEntryCard from "./PlanEntryCard";
import EscalationCard from "./EscalationCard";
import Spinner from "./Spinner";

function applyBadge(entry: PlanEntry, applyItems: ApplyItem[]): ReactNode {
  if (entry.action === "ok") return null;
  const item = applyItems.find((a) => a?.label === entry.spec.label);
  const verb = (action: string, ing: boolean) =>
    action === "create"
      ? ing ? "Creating" : "Created"
      : action === "delete"
        ? ing ? "Deleting" : "Deleted"
        : ing ? "Updating" : "Updated";

  if (!item) {
    return <span className="text-xs text-gray-400">Waiting</span>;
  }
  if (item.status === "done") {
    return <span className="text-xs text-green-600">{verb(item.action, false)}</span>;
  }
  return (
    <span className="flex items-center gap-1.5 text-xs text-blue-600">
      <Spinner className="h-3 w-3" />
      {verb(item.action, true)}
    </span>
  );
}

/**
 * The resources this scan could not read.
 *
 * Above the plan rather than only in the cards: the plan is incomplete, and
 * the operator is about to decide whether to apply it. "In sync" over a
 * resource nobody could see is the claim this notice exists to stop.
 */
function UnreadableNotice({ entries }: { entries: PlanEntry[] }) {
  return (
    <div className="bg-gray-50 border border-gray-300 rounded-lg p-4">
      <p className="text-sm font-medium text-gray-800">
        {entries.length} resource{entries.length !== 1 ? "s" : ""} could not be
        checked
      </p>
      <p className="text-sm text-gray-600 mt-1">
        AWS refused or failed these reads, so this plan does not describe them.
        They may be missing, or configured differently from what Claria expects
        — usually the credentials in use are missing a permission.
      </p>
      <ul className="mt-2 space-y-0.5">
        {entries.map((e) => (
          <li key={e.spec.resource_name} className="text-sm text-gray-700">
            {e.spec.label}
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * The whole plan as one flat list in manifest order — every resource visible,
 * drift diffs inline in the cards. During apply, `applyItems` adds a per-row
 * progress badge.
 */
export default function PlanView({
  entries,
  showEscalationNotice,
  applyItems,
}: {
  entries: PlanEntry[];
  showEscalationNotice?: boolean;
  applyItems?: ApplyItem[];
}) {
  const total = entries.length;
  const unreadable = unreadableEntries(entries);
  const changesCount = entries.filter(
    (e) => e.action !== "ok" && e.action !== "unknown"
  ).length;

  return (
    <div className="space-y-4">
      {/* Summary bar */}
      <p className="text-sm text-gray-600">
        {total} resource{total !== 1 ? "s" : ""} —{" "}
        {applyItems
          ? `applying ${changesCount} change${changesCount !== 1 ? "s" : ""}...`
          : changesCount > 0
            ? `${changesCount} change${changesCount !== 1 ? "s" : ""} needed`
            : unreadable.length > 0
              ? "everything Claria could check is in sync"
              : "all resources in sync"}
      </p>

      {unreadable.length > 0 && <UnreadableNotice entries={unreadable} />}

      {showEscalationNotice && <EscalationCard />}

      <div className="space-y-2">
        {entries.map((entry, i) => (
          <PlanEntryCard
            key={`${entry.spec.resource_name}-${i}`}
            entry={entry}
            trailing={applyItems ? applyBadge(entry, applyItems) : undefined}
          />
        ))}
      </div>
    </div>
  );
}
