import HistoryFolder from "./HistoryFolder";
import { formatDateTime } from "../lib/format";
import { type EditorHistoryEntry } from "../lib/tauri";
import { PlayIcon } from "./icons";

/** Persisted Writing sessions, collapsed into one folder row. */
export default function EditorHistoryFolder({
  entries,
  onResume,
}: {
  entries: EditorHistoryEntry[];
  onResume: (reportId: string) => void;
}) {
  if (entries.length === 0) return null;

  return (
    <HistoryFolder
      title="Editor History"
      subtitle={`${entries.length} writing session${entries.length !== 1 ? "s" : ""}`}
      accent="blue"
    >
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
              {entry.name}
            </p>
            <p className="text-xs text-gray-500 truncate">{entry.title}</p>
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
    </HistoryFolder>
  );
}
