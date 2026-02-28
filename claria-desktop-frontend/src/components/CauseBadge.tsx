import type { Cause } from "../lib/tauri";

const labels: Record<Cause, string> = {
  in_sync: "",
  first_provision: "First-time setup",
  drift: "Configuration drift detected",
  manifest_changed: "New in this Claria update",
  orphaned: "No longer managed — will be removed",
};

export default function CauseBadge({ cause }: { cause: Cause }) {
  if (cause === "in_sync") return null;
  return <span className="text-xs text-gray-500 mt-1 block">{labels[cause]}</span>;
}
