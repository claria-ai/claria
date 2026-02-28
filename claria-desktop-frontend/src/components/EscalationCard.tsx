import type { PlanEntry } from "../lib/tauri";
import FieldDriftList from "./FieldDriftList";

export default function EscalationCard({
  entry,
  onEscalate,
}: {
  entry: PlanEntry;
  onEscalate: () => void;
}) {
  return (
    <div className="border border-amber-300 bg-amber-50 rounded-lg p-4">
      <p className="text-amber-900 font-medium text-sm">
        Permission Update Required
      </p>
      <p className="text-amber-800 text-sm mt-1">
        This Claria update needs additional AWS permissions that your current
        IAM policy doesn't include. Temporary elevated credentials (root or
        admin) will be used once to update the policy and then discarded.
      </p>
      {entry.drift.length > 0 && (
        <details className="mt-2 text-xs">
          <summary className="cursor-pointer text-amber-700">
            Show missing permissions
          </summary>
          <FieldDriftList drifts={entry.drift} />
        </details>
      )}
      <button
        onClick={onEscalate}
        className="mt-3 px-4 py-2 bg-amber-600 text-white text-sm rounded-lg hover:bg-amber-700 transition-colors"
      >
        Provide Elevated Credentials
      </button>
    </div>
  );
}
