import { memo } from "react";
import type { ReportTimelineItemView } from "../lib/tauri";
import MessageBubble from "./MessageBubble";

/**
 * One writer-timeline entry: either a conversation message (shared chat
 * bubble) or a collapsed raw tool invocation for auditability.
 */
const TimelineItem = memo(function TimelineItem({
  item,
}: {
  item: ReportTimelineItemView;
}) {
  if (item.kind === "tool_activity") {
    const color =
      item.status === "failed"
        ? "border-red-200 bg-red-50 text-red-700"
        : item.status === "succeeded"
          ? "border-emerald-200 bg-emerald-50 text-emerald-700"
          : "border-gray-200 bg-gray-50 text-gray-600";
    return (
      <details className={`group border rounded-md text-xs ${color}`}>
        <summary className="list-none cursor-pointer px-3 py-2 flex items-center gap-2 [&::-webkit-details-marker]:hidden">
          <span
            aria-hidden="true"
            className="inline-block transition-transform group-open:rotate-90"
          >
            ▸
          </span>
          <span className="flex-1">{item.summary}</span>
          <code className="hidden sm:inline text-[10px] opacity-70">
            {item.name}
          </code>
          <span className="capitalize">{item.status}</span>
        </summary>
        <div className="border-t border-current/15 bg-gray-950 text-gray-100 px-3 py-3 space-y-3 select-text">
          <div>
            <p className="mb-1 text-[10px] uppercase tracking-wide text-gray-400">
              Raw LLM invocation
            </p>
            <pre className="max-h-72 overflow-auto whitespace-pre text-[11px] leading-4 font-mono">
              {item.invocation_json}
            </pre>
          </div>
          {item.result_json && (
            <div>
              <p className="mb-1 text-[10px] uppercase tracking-wide text-gray-400">
                Correlated tool result
              </p>
              <pre className="max-h-72 overflow-auto whitespace-pre text-[11px] leading-4 font-mono">
                {item.result_json}
              </pre>
            </div>
          )}
        </div>
      </details>
    );
  }

  return <MessageBubble role={item.role} content={item.text} />;
});

export default TimelineItem;
