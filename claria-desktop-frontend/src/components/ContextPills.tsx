import type { ReportWorkspaceView } from "../lib/tauri";
import type { WritingBlockReference } from "../lib/writingComposerDraft";

export type ContextPill = {
  key: string;
  label: string;
  status: "loading" | "ready" | "failed";
  filename?: string;
};

/**
 * Assemble the writer's context pills from the workspace, the queued report
 * references, and the live tool activity of an in-flight turn.
 */
export function buildContextPills(
  workspace: ReportWorkspaceView,
  references: WritingBlockReference[],
  liveContext: ContextPill[]
): ContextPill[] {
  const contextReads = workspace.turns.flatMap((turn) => turn.context_reads);
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
  for (const filename of new Set(contextReads.map((read) => read.filename))) {
    pills.push({
      key: `record:${filename}`,
      label: filename,
      status: "ready",
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

export default function ContextPills({
  pills,
  onPreviewFile,
}: {
  pills: ContextPill[];
  onPreviewFile: (filename: string) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1.5" aria-label="Writer context">
      {pills.map((pill) => {
        const className = `inline-flex max-w-full items-center gap-1 rounded-full border px-2 py-1 text-[10px] font-medium ${
          pill.status === "failed"
            ? "border-red-200 bg-red-50 text-red-700"
            : pill.status === "loading"
              ? "border-blue-200 bg-blue-50 text-blue-700"
              : "border-emerald-200 bg-emerald-50 text-emerald-700"
        }`;
        const content = (
          <>
            <span
              aria-hidden="true"
              className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                pill.status === "failed"
                  ? "bg-red-500"
                  : pill.status === "loading"
                    ? "bg-blue-500 animate-pulse"
                    : "bg-emerald-500"
              }`}
            />
            <span className="truncate">{pill.label}</span>
          </>
        );
        return pill.filename ? (
          <button
            type="button"
            key={pill.key}
            onClick={() => onPreviewFile(pill.filename!)}
            title={`Preview ${pill.filename}`}
            className={`${className} hover:border-emerald-400 hover:text-emerald-900`}
          >
            {content}
          </button>
        ) : (
          <span key={pill.key} className={className}>
            {content}
          </span>
        );
      })}
    </div>
  );
}
