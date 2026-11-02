import type { RecordFile } from "./tauri";

// Session-only coordination. No jobs, timestamps, or processing registry is persisted.
export type ProcessingFile = RecordFile & { originalSaved?: boolean };
type Snapshot = { failures: ProcessingFile[]; busy: ReadonlySet<string> };
const empty: Snapshot = { failures: [], busy: new Set() };
const snapshots = new Map<string, Snapshot>();
const listeners = new Map<string, Set<() => void>>();
export function processingSnapshot(scope: string): Snapshot { return snapshots.get(scope) ?? empty; }
export function subscribeProcessing(scope: string, listener: () => void) {
  const scoped = listeners.get(scope) ?? new Set();
  scoped.add(listener);
  listeners.set(scope, scoped);
  return () => { scoped.delete(listener); if (!scoped.size) listeners.delete(scope); };
}
function publish(scope: string, next: Snapshot) {
  if (!next.failures.length && !next.busy.size) snapshots.delete(scope);
  else snapshots.set(scope, next);
  listeners.get(scope)?.forEach((listener) => listener());
}
export function beginProcessing(scope: string, filename: string): boolean {
  const current = processingSnapshot(scope);
  if (current.busy.has(filename)) return false;
  publish(scope, { ...current, busy: new Set([...current.busy, filename]) });
  return true;
}
export function finishProcessing(scope: string, filename: string, file?: ProcessingFile) {
  const current = processingSnapshot(scope);
  const busy = new Set(current.busy);
  busy.delete(filename);
  const failures = current.failures.filter((entry) => entry.filename !== filename);
  if (file?.processing_status?.status === "failed" || file?.processing_status?.status === "unknown") failures.push(file);
  publish(scope, { failures, busy });
}
export function reviewProcessing(scope: string, file: RecordFile) {
  const current = processingSnapshot(scope);
  publish(scope, { ...current, failures: [...current.failures.filter((entry) => entry.filename !== file.filename), file] });
}
