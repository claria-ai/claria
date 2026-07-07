import type { ReactNode } from "react";
import type { PlanEntry } from "../lib/tauri";
import type { ApplyItem } from "../lib/provisioner";
import PlanEntryCard from "./PlanEntryCard";
import EscalationCard from "./EscalationCard";
import Spinner from "./Spinner";
import { findEscalationEntry } from "../lib/plan";

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
 * The whole plan as one flat list in manifest order — every resource visible,
 * drift diffs inline in the cards. During apply, `applyItems` adds a per-row
 * progress badge.
 */
export default function PlanView({
  entries,
  onEscalate,
  applyItems,
}: {
  entries: PlanEntry[];
  onEscalate?: () => void;
  applyItems?: ApplyItem[];
}) {
  const total = entries.length;
  const changesCount = entries.filter((e) => e.action !== "ok").length;

  // IAM escalation gets a CTA banner; its entry is omitted from the list
  // below since the banner already shows its drift.
  const escalation = onEscalate ? findEscalationEntry(entries) : null;

  return (
    <div className="space-y-4">
      {/* Summary bar */}
      <p className="text-sm text-gray-600">
        {total} resource{total !== 1 ? "s" : ""} —{" "}
        {applyItems
          ? `applying ${changesCount} change${changesCount !== 1 ? "s" : ""}...`
          : changesCount > 0
            ? `${changesCount} change${changesCount !== 1 ? "s" : ""} needed`
            : "all resources in sync"}
      </p>

      {escalation && onEscalate && (
        <EscalationCard entry={escalation} onEscalate={onEscalate} />
      )}

      <div className="space-y-2">
        {entries
          .filter((e) => e !== escalation)
          .map((entry, i) => (
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
