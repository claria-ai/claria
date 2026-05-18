import type { FieldDrift } from "../lib/tauri";

type Primitive = string | number | boolean;

function pretty(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function isPrimitiveArray(v: unknown): v is Primitive[] {
  return (
    Array.isArray(v) &&
    v.every(
      (x) =>
        typeof x === "string" ||
        typeof x === "number" ||
        typeof x === "boolean",
    )
  );
}

type DiffRow =
  | { kind: "unchanged"; value: Primitive }
  | { kind: "added"; value: Primitive }
  | { kind: "removed"; value: Primitive };

function computeArrayDiff(
  expected: Primitive[],
  actual: Primitive[],
): DiffRow[] {
  const expectedSet = new Set(expected);
  const actualSet = new Set(actual);
  const union = new Set<Primitive>([...expected, ...actual]);
  const sorted = [...union].sort((a, b) => {
    const sa = String(a);
    const sb = String(b);
    return sa < sb ? -1 : sa > sb ? 1 : 0;
  });
  return sorted.map((value) => {
    const inExpected = expectedSet.has(value);
    const inActual = actualSet.has(value);
    if (inExpected && inActual) return { kind: "unchanged", value };
    if (inExpected) return { kind: "added", value };
    return { kind: "removed", value };
  });
}

function ArrayDiff({
  expected,
  actual,
}: {
  expected: Primitive[];
  actual: Primitive[];
}) {
  const rows = computeArrayDiff(expected, actual);
  return (
    <pre className="whitespace-pre-wrap mt-1 bg-gray-50 border border-gray-200 rounded p-2">
      {rows.map((row, i) => {
        if (row.kind === "unchanged") {
          return (
            <div key={i} className="text-gray-500">
              {"  "}
              {String(row.value)}
            </div>
          );
        }
        if (row.kind === "added") {
          return (
            <div key={i} className="text-green-700">
              {"+ "}
              {String(row.value)}
            </div>
          );
        }
        return (
          <div key={i} className="text-red-600 line-through">
            {"- "}
            {String(row.value)}
          </div>
        );
      })}
    </pre>
  );
}

function FallbackDrift({ drift }: { drift: FieldDrift }) {
  return (
    <>
      <pre className="text-red-600 line-through whitespace-pre-wrap mt-1">
        {pretty(drift.actual)}
      </pre>
      <pre className="text-green-700 whitespace-pre-wrap mt-1">
        {pretty(drift.expected)}
      </pre>
    </>
  );
}

export default function FieldDriftList({ drifts }: { drifts: FieldDrift[] }) {
  if (drifts.length === 0) return null;
  return (
    <div className="mt-2 space-y-1">
      {drifts.map((d) => (
        <div key={d.field} className="text-xs font-mono">
          <span className="text-gray-500">{d.label}:</span>
          {isPrimitiveArray(d.expected) && isPrimitiveArray(d.actual) ? (
            <ArrayDiff expected={d.expected} actual={d.actual} />
          ) : (
            <FallbackDrift drift={d} />
          )}
        </div>
      ))}
    </div>
  );
}
