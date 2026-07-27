import Spinner from "./Spinner";
import { FolderIcon, PlayIcon, TrashIcon } from "./icons";
import { formatFileSize } from "../lib/format";
import { chatHistoryLabel } from "../lib/recordFiles";
import type { RecordFile } from "../lib/tauri";

/**
 * Persisted chat sessions, collapsed into one folder row.
 *
 * They are ordinary objects in the record and would otherwise crowd out the
 * user's own documents, so they get folded away rather than listed inline.
 */
export default function ChatHistoryFolder({
  files,
  open,
  onToggle,
  resumeLoading,
  onResume,
  onDelete,
}: {
  files: RecordFile[];
  open: boolean;
  onToggle: () => void;
  /** Filename of the chat currently being loaded, if any. */
  resumeLoading: string | null;
  onResume: (filename: string) => void;
  onDelete: (filename: string) => void;
}) {
  return (
    <div className="border-b border-gray-100">
      <button
        onClick={onToggle}
        className="w-full px-4 py-3 flex items-center gap-3 hover:bg-gray-50 transition-colors"
      >
        <div className="w-8 h-8 rounded flex items-center justify-center bg-purple-100 text-purple-600">
          <FolderIcon />
        </div>
        <div className="flex-1 min-w-0 text-left">
          <p className="text-sm font-medium text-gray-900">Chat History</p>
          <p className="text-xs text-gray-400">
            {files.length} conversation{files.length !== 1 ? "s" : ""}
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
          {files.map((file) => (
            <div
              key={file.filename}
              className="px-4 py-3 pl-8 flex items-center gap-3"
            >
              <div className="w-8 h-8 rounded flex items-center justify-center bg-purple-50 text-purple-500 text-xs font-bold">
                AI
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-gray-900 truncate">
                  {chatHistoryLabel(file.filename)}
                </p>
                <p className="text-xs text-gray-400">
                  {formatFileSize(file.size)}
                </p>
              </div>
              <div className="flex gap-1">
                <button
                  onClick={() => onResume(file.filename)}
                  disabled={resumeLoading === file.filename}
                  title="Resume conversation"
                  className="p-1.5 text-gray-400 hover:text-purple-600 hover:bg-purple-50 rounded transition-colors disabled:opacity-50"
                >
                  {resumeLoading === file.filename ? <Spinner /> : <PlayIcon />}
                </button>
                <button
                  onClick={() => onDelete(file.filename)}
                  title="Delete chat history"
                  className="p-1.5 text-gray-400 hover:text-red-600 hover:bg-red-50 rounded transition-colors"
                >
                  <TrashIcon />
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
