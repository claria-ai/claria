import type { FieldDrift } from "../lib/tauri";

function pretty(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

export default function FieldDriftList({ drifts }: { drifts: FieldDrift[] }) {
  if (drifts.length === 0) return null;
  return (
    <div className="mt-2 space-y-1">
      {drifts.map((d) => (
        <div key={d.field} className="text-xs font-mono">
          <span className="text-gray-500">{d.label}:</span>
          <pre className="text-red-600 line-through whitespace-pre-wrap mt-1">
            {pretty(d.actual)}
          </pre>
          <pre className="text-green-700 whitespace-pre-wrap mt-1">
            {pretty(d.expected)}
          </pre>
        </div>
      ))}
    </div>
  );
}
