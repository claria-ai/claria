import { useState } from "react";
import { formatDateTime } from "../lib/format";
import { type EditorHistoryEntry } from "../lib/tauri";
import { FolderIcon, PlayIcon } from "./icons";

/** Persisted Writing sessions, collapsed into one folder row. */
export default function EditorHistoryFolder({
  entries,
  onResume,
}: {
  entries: EditorHistoryEntry[];
  onResume: (reportId: string) => void;
}) {
  const [open, setOpen] = useState(false);

  if (entries.length === 0) return null;

  return (
    <div className="border-b border-gray-100">
      <button
        onClick={() => setOpen((value) => !value)}
        className="w-full px-4 py-3 flex items-center gap-3 hover:bg-gray-50 transition-colors"
      >
        <div className="w-8 h-8 rounded flex items-center justify-center bg-blue-100 text-blue-600">
          <FolderIcon />
        </div>
        <div className="flex-1 min-w-0 text-left">
          <p className="text-sm font-medium text-gray-900">Editor History</p>
          <p className="text-xs text-gray-400">
            {entries.length} writing session{entries.length !== 1 ? "s" : ""}
          </p>
        </div>
        <svg
          className={`w-4 h-4 text-gray-400 transition-transform ${
            open ? "rotate-90" : ""
          }`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 5l7 7-7 7"
          />
        </svg>
      </button>
      {open && (
        <div className="divide-y divide-gray-100 bg-gray-50/50">
          {entries.map((entry) => (
            <div
              key={entry.report_id}
              className="px-4 py-3 pl-8 flex items-center gap-3"
            >
              <div className="w-8 h-8 rounded flex items-center justify-center bg-blue-50 text-blue-600 text-xs font-bold">
                W
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-gray-900 truncate">
                  {entry.title}
                </p>
                <p className="text-xs text-gray-400">
                  Revision {entry.revision} · {entry.turn_count} turn
                  {entry.turn_count === 1 ? "" : "s"} ·{" "}
                  {formatDateTime(entry.updated_at)}
                </p>
                {entry.last_export && (
                  <p className="text-[10px] text-gray-400 capitalize">
                    Word export {entry.last_export.status} · revision{" "}
                    {entry.last_export.revision}
                  </p>
                )}
              </div>
              <button
                onClick={() => onResume(entry.report_id)}
                title="Resume writing session"
                className="p-1.5 text-gray-400 hover:text-blue-600 hover:bg-blue-50 rounded transition-colors"
              >
                <PlayIcon />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
