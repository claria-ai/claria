import type { ReportWorkspaceView } from "./tauri";
import type { WritingBlockReference } from "./writingComposerDraft";

export type ContextPill = {
  key: string;
  label: string;
  status: "loading" | "ready" | "failed";
  filename?: string;
};

/**
 * Assemble the writer's context pills from report history, eager record
 * snapshots, later tool reads, queued references, and in-flight activity.
 */
export function buildContextPills(
  workspace: ReportWorkspaceView,
  references: WritingBlockReference[],
  liveContext: ContextPill[]
): ContextPill[] {
  const contextReads = workspace.turns.flatMap((turn) => turn.context_reads);
  const preloadedFiles = workspace.turns.flatMap(
    (turn) => turn.context_files ?? []
  );
  const pills: ContextPill[] = [
    {
      key: "accepted-report",
      label: `Accepted report · r${workspace.draft.revision}`,
      status: "ready",
    },
  ];
  if (workspace.turns.length > 0) {
    pills.push({
      key: "session-history",
      label: `${workspace.turns.length} prior turn${workspace.turns.length === 1 ? "" : "s"}`,
      status: "ready",
    });
  }
  if (workspace.template_import) {
    pills.push({ key: "template", label: "Template provenance", status: "ready" });
  }
  if (
    workspace.turns.some((turn) =>
      turn.timeline.some(
        (item) => item.kind === "tool_activity" && item.name === "list_record_files"
      )
    )
  ) {
    pills.push({ key: "record-list", label: "Record file list", status: "ready" });
  }
  // Whole-report generation records every file in its eager source snapshot,
  // while later targeted turns expose successful tool reads. Merge both so a
  // newly uploaded file appears as soon as Claude actually reads it, and a
  // successful read upgrades an earlier unavailable snapshot entry.
  const recordFiles = new Map<string, ContextPill["status"]>();
  for (const file of preloadedFiles) {
    const status = file.available ? "ready" : "failed";
    if (status === "ready" || !recordFiles.has(file.filename)) {
      recordFiles.set(file.filename, status);
    }
  }
  for (const read of contextReads) recordFiles.set(read.filename, "ready");
  for (const [filename, status] of recordFiles) {
    pills.push({
      key: `record:${filename}`,
      label: filename,
      status,
      filename,
    });
  }
  for (const reference of references) {
    pills.push({
      key: `reference:${reference.sectionId}:${reference.blockIndex}`,
      label: `${reference.sectionHeading} · ${reference.kind}`,
      status: "ready",
    });
  }
  for (const live of liveContext) {
    const existing = pills.find((pill) => pill.label === live.label);
    if (existing) {
      existing.status = live.status;
      existing.filename ??= live.filename;
    } else pills.push(live);
  }
  return pills;
}

/** Upsert a live tool-activity pill by its context label. */
export function upsertLiveContext(
  current: ContextPill[],
  label: string,
  status: "loading" | "ready" | "failed"
): ContextPill[] {
  const key = `live:${label}`;
  const existing = current.find((item) => item.key === key);
  if (existing) {
    return current.map((item) => (item.key === key ? { ...item, status } : item));
  }
  return [...current, { key, label, status, filename: label }];
}
