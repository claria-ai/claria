import type { Cause } from "../lib/tauri";

const labels: Record<Cause, string> = {
  in_sync: "",
  missing: "Not yet provisioned",
  drift: "Configuration drift detected",
  orphaned: "No longer managed — will be removed",
};

export default function CauseBadge({ cause }: { cause: Cause }) {
  if (cause === "in_sync") return null;
  return <span className="text-xs text-gray-500 mt-1 block">{labels[cause]}</span>;
}
