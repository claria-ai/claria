import type { PlanEntry, Action } from "../lib/tauri";
import CauseBadge from "./CauseBadge";
import FieldDriftList from "./FieldDriftList";

const actionStyles: Record<Action, { icon: string; border: string }> = {
  ok: { icon: "\u2705", border: "border-green-200" },
  create: { icon: "\uD83C\uDD95", border: "border-blue-200" },
  modify: { icon: "\uD83D\uDD27", border: "border-amber-200" },
  delete: { icon: "\uD83D\uDDD1\uFE0F", border: "border-red-200" },
  precondition_failed: { icon: "\uD83D\uDD12", border: "border-amber-300" },
};

export default function PlanEntryCard({ entry }: { entry: PlanEntry }) {
  const style = actionStyles[entry.action];
  return (
    <div className={`border ${style.border} rounded-lg`}>
      <div className="flex items-start gap-3 p-4">
        <span className="shrink-0 mt-0.5">{style.icon}</span>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="font-medium text-sm text-gray-800">
              {entry.spec.label}
            </span>
            <span className="text-xs font-mono text-gray-400">
              {entry.spec.resource_name}
            </span>
          </div>
          <p className="text-sm text-gray-600 mt-0.5">
            {entry.spec.description}
          </p>
          <CauseBadge cause={entry.cause} />
          <FieldDriftList drifts={entry.drift} />
        </div>
      </div>
    </div>
  );
}
