import type { ReactNode } from "react";
import type { PlanEntry } from "../lib/tauri";
import type { ApplyItem, ScanItem } from "../lib/provisioner";
import PlanView from "./PlanView";
import Spinner from "./Spinner";
import { hasChanges } from "../lib/plan";

export type InfraPhase = "scanning" | "planned" | "applying" | "done" | "error";

function StepChecklist(
  props:
    | { variant: "scan"; items: ScanItem[] }
    | { variant: "apply"; items: ApplyItem[] }
) {
  const rows =
    props.variant === "scan"
      ? props.items
          .filter((i) => i.status !== "pending")
          .map((item) => ({
            label: item.label,
            state: item.status === "scanning" ? ("active" as const) : ("done" as const),
            rowClass: item.status === "scanning" ? "text-blue-700" : "text-gray-500",
            trailing: null as ReactNode,
          }))
      : props.items.map((item) => ({
          label: item.label,
          state:
            item.status === "in_progress"
              ? ("active" as const)
              : item.status === "done"
                ? ("done" as const)
                : ("pending" as const),
          rowClass:
            item.status === "pending"
              ? "text-gray-400"
              : item.status === "in_progress"
                ? "text-blue-700"
                : "text-gray-600",
          trailing: (item.status === "in_progress" ? (
            <span className="text-xs text-blue-500 ml-auto">
              {item.action === "create" ? "Creating" : "Updating"}
            </span>
          ) : item.status === "done" ? (
            <span className="text-xs text-gray-400 ml-auto">
              {item.action === "create" ? "Created" : "Updated"}
            </span>
          ) : null) as ReactNode,
        }));

  return (
    <div className="bg-blue-50 border border-blue-200 rounded-lg p-4">
      <p className="text-sm font-medium text-blue-800 mb-3">
        {props.variant === "scan" ? "Scanning AWS resources..." : "Applying changes..."}
      </p>
      <div className="space-y-1.5">
        {rows.map((row) => (
          <div
            key={row.label}
            className={`flex items-center gap-2 text-sm transition-opacity duration-300 ${row.rowClass}`}
          >
            {row.state === "active" ? (
              <Spinner className="h-3.5 w-3.5 shrink-0" />
            ) : row.state === "done" ? (
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
            ) : (
              <svg
                className="h-3.5 w-3.5 shrink-0"
                viewBox="0 0 20 20"
                fill="currentColor"
              >
                <circle cx="10" cy="10" r="4" />
              </svg>
            )}
            <span>{row.label}</span>
            {row.trailing}
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
  onEscalate,
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
  onEscalate?: () => void;
  actions?: ReactNode;
  errorActions?: ReactNode;
  doneMessage?: string;
  showInSync?: boolean;
}) {
  if (phase === "scanning") {
    return <StepChecklist variant="scan" items={scanItems} />;
  }

  if (phase === "planned") {
    if (!entries) return null;
    return (
      <div className="space-y-4">
        <PlanView entries={entries} onEscalate={onEscalate} />

        {showInSync && !hasChanges(entries) && (
          <p className="text-sm text-green-600 text-center py-2">
            All resources are in sync.
          </p>
        )}

        {actions}
      </div>
    );
  }

  if (phase === "applying") {
    return (
      <div className="space-y-4">
        {scanItems.length > 0 && <StepChecklist variant="scan" items={scanItems} />}
        {applyItems.length > 0 && <StepChecklist variant="apply" items={applyItems} />}
        {scanItems.length === 0 && applyItems.length === 0 && (
          <div className="flex items-center justify-center py-8">
            <Spinner className="h-6 w-6 text-blue-500" />
          </div>
        )}
      </div>
    );
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
