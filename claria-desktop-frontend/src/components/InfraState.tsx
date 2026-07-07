import type { ReactNode } from "react";
import type { PlanEntry } from "../lib/tauri";
import type { ApplyItem, ScanItem } from "../lib/provisioner";
import PlanView from "./PlanView";
import Spinner from "./Spinner";
import { hasChanges } from "../lib/plan";

export type InfraPhase = "scanning" | "planned" | "applying" | "done" | "error";

/**
 * Scan progress rendered as the same card list the plan fills in, so rows
 * persist across the scanning → planned transition instead of the checklist
 * being swapped out for a differently-shaped view.
 */
function ScanList({ items }: { items: ScanItem[] }) {
  return (
    <div className="space-y-4">
      <p className="text-sm text-gray-600">Scanning AWS resources...</p>
      <div className="space-y-2">
        {items
          .filter((i) => i.status !== "pending")
          .map((item) => (
            <div
              key={item.label}
              className={`border rounded-lg flex items-center gap-3 p-4 transition-opacity duration-300 ${
                item.status === "scanning" ? "border-blue-200" : "border-gray-200"
              }`}
            >
              {item.status === "scanning" ? (
                <Spinner className="h-3.5 w-3.5 shrink-0 text-blue-500" />
              ) : (
                <svg
                  className="h-3.5 w-3.5 shrink-0 text-green-500"
                  viewBox="0 0 20 20"
                  fill="currentColor"
                >
                  <path
                    fillRule="evenodd"
                    d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z"
                    clipRule="evenodd"
                  />
                </svg>
              )}
              <span
                className={`text-sm font-medium ${
                  item.status === "scanning" ? "text-blue-700" : "text-gray-800"
                }`}
              >
                {item.label}
              </span>
            </div>
          ))}
      </div>
    </div>
  );
}

export default function InfraState({
  phase,
  entries,
  scanItems,
  applyItems,
  error,
  showEscalationNotice,
  actions,
  errorActions,
  doneMessage = "All resources provisioned successfully.",
  showInSync = true,
}: {
  phase: InfraPhase;
  entries: PlanEntry[] | null;
  scanItems: ScanItem[];
  applyItems: ApplyItem[];
  error?: string | null;
  showEscalationNotice?: boolean;
  actions?: ReactNode;
  errorActions?: ReactNode;
  doneMessage?: string;
  showInSync?: boolean;
}) {
  if (phase === "scanning") {
    return <ScanList items={scanItems} />;
  }

  if (phase === "planned") {
    if (!entries) return null;
    return (
      <div className="space-y-4">
        <PlanView entries={entries} showEscalationNotice={showEscalationNotice} />

        {showInSync && !hasChanges(entries) && (
          <p className="text-sm text-green-600 text-center py-2">
            All resources are in sync.
          </p>
        )}

        {/* Scroll target for the escalation notice's anchor link */}
        <div id="infra-plan-actions">{actions}</div>
      </div>
    );
  }

  if (phase === "applying") {
    // Destroy runs without a progress channel and with a stale plan — no
    // entries worth showing, so fall back to a bare spinner.
    if (!entries || (scanItems.length === 0 && applyItems.length === 0)) {
      return (
        <div className="flex items-center justify-center py-8">
          <Spinner className="h-6 w-6 text-blue-500" />
        </div>
      );
    }
    return <PlanView entries={entries} applyItems={applyItems} />;
  }

  if (phase === "done") {
    return (
      <div className="space-y-4">
        <div className="bg-green-50 border border-green-200 rounded-lg p-4 text-center">
          <p className="text-sm font-medium text-green-800">{doneMessage}</p>
        </div>

        {entries && <PlanView entries={entries} />}

        {actions}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="bg-red-50 border border-red-200 rounded-lg p-4">
        <p className="text-sm font-medium text-red-800 mb-1">Error</p>
        <p className="text-xs text-red-700 font-mono whitespace-pre-wrap">
          {error}
        </p>
      </div>

      {errorActions}
    </div>
  );
}
