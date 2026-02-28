import type { PlanEntry } from "../lib/tauri";
import PlanEntryCard from "./PlanEntryCard";
import EscalationCard from "./EscalationCard";

/** Check whether a plan has any actionable entries. */
export function hasChanges(entries: PlanEntry[] | null): boolean {
  if (!entries) return false;
  return entries.some((e) => e.action !== "ok");
}

export default function PlanView({
  entries,
  onEscalate,
}: {
  entries: PlanEntry[];
  onEscalate?: () => void;
}) {
  const total = entries.length;
  const ready = entries.filter((e) => e.action === "ok");
  const needsAttention = entries.filter(
    (e) =>
      (e.action === "precondition_failed" ||
        e.spec.severity === "elevated" ||
        e.spec.severity === "destructive") &&
      e.action !== "ok"
  );
  const changes = entries.filter(
    (e) =>
      (e.action === "create" || e.action === "modify" || e.action === "delete") &&
      !needsAttention.includes(e)
  );

  // Detect IAM escalation: iam_user_policy precondition failed + manifest changed
  const escalation = entries.find(
    (e) =>
      e.spec.resource_type === "iam_user_policy" &&
      e.action === "precondition_failed" &&
      e.cause === "manifest_changed"
  );

  const changesCount =
    needsAttention.length + changes.length;

  return (
    <div className="space-y-4">
      {/* Summary bar */}
      <p className="text-sm text-gray-600">
        {total} resource{total !== 1 ? "s" : ""} —{" "}
        {changesCount > 0
          ? `${changesCount} change${changesCount !== 1 ? "s" : ""} needed`
          : "all resources in sync"}
      </p>

      {/* Needs Attention section */}
      {needsAttention.length > 0 && (
        <div>
          <h4 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-2">
            Needs Attention ({needsAttention.length})
          </h4>
          <div className="space-y-2">
            {escalation && onEscalate && (
              <EscalationCard entry={escalation} onEscalate={onEscalate} />
            )}
            {needsAttention
              .filter((e) => e !== escalation)
              .map((entry, i) => (
                <PlanEntryCard key={`attn-${i}`} entry={entry} />
              ))}
          </div>
        </div>
      )}

      {/* Changes section */}
      {changes.length > 0 && (
        <div>
          <h4 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-2">
            Changes ({changes.length})
          </h4>
          <div className="space-y-2">
            {changes.map((entry, i) => (
              <PlanEntryCard key={`change-${i}`} entry={entry} />
            ))}
          </div>
        </div>
      )}

      {/* Ready section — collapsed by default */}
      {ready.length > 0 && (
        <details>
          <summary className="text-xs font-semibold text-gray-500 uppercase tracking-wide cursor-pointer">
            Ready ({ready.length})
          </summary>
          <div className="space-y-2 mt-2">
            {ready.map((entry, i) => (
              <PlanEntryCard key={`ready-${i}`} entry={entry} />
            ))}
          </div>
        </details>
      )}
    </div>
  );
}
