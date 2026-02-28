import type { FieldDrift } from "../lib/tauri";

export default function FieldDriftList({ drifts }: { drifts: FieldDrift[] }) {
  if (drifts.length === 0) return null;
  return (
    <div className="mt-2 space-y-1">
      {drifts.map((d) => (
        <div key={d.field} className="text-xs font-mono">
          <span className="text-gray-500">{d.label}:</span>{" "}
          <span className="text-red-600 line-through">
            {JSON.stringify(d.actual)}
          </span>{" "}
          <span className="text-green-700">
            {JSON.stringify(d.expected)}
          </span>
        </div>
      ))}
    </div>
  );
}
