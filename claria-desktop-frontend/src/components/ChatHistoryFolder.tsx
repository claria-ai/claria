import { useState } from "react";
import Spinner from "./Spinner";
import { FolderIcon, PlayIcon, TrashIcon } from "./icons";
import { formatDateTime, formatFileSize } from "../lib/format";
import {
  loadChatHistory,
  type ChatHistoryDetail,
  type ChatHistorySummary,
} from "../lib/tauri";

/** Persisted chat sessions, collapsed into one self-contained folder row. */
export default function ChatHistoryFolder({
  clientId,
  entries,
  onResume,
  onDelete,
  onError,
}: {
  clientId: string;
  entries: ChatHistorySummary[];
  onResume: (detail: ChatHistoryDetail) => void;
  onDelete: (filename: string) => void;
  onError: (message: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [resumeLoading, setResumeLoading] = useState<string | null>(null);

  async function handleResume(entry: ChatHistorySummary) {
    setResumeLoading(entry.filename);
    try {
      onResume(await loadChatHistory(clientId, entry.chat_id));
    } catch (error) {
      onError(String(error));
    } finally {
      setResumeLoading(null);
    }
  }

  return (
    <div className="border-b border-gray-100">
      <button
        onClick={() => setOpen((value) => !value)}
        className="w-full px-4 py-3 flex items-center gap-3 hover:bg-gray-50 transition-colors"
      >
        <div className="w-8 h-8 rounded flex items-center justify-center bg-purple-100 text-purple-600">
          <FolderIcon />
        </div>
        <div className="flex-1 min-w-0 text-left">
          <p className="text-sm font-medium text-gray-900">Chat History</p>
          <p className="text-xs text-gray-400">
            {entries.length} conversation{entries.length !== 1 ? "s" : ""}
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
              key={entry.chat_id}
              className="px-4 py-3 pl-8 flex items-center gap-3"
            >
              <div className="w-8 h-8 rounded flex items-center justify-center bg-purple-50 text-purple-500 text-xs font-bold">
                AI
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-gray-900 truncate">
                  {entry.name}
                </p>
                <p className="text-xs text-gray-400">
                  {formatDateTime(entry.updated_at)} · {formatFileSize(entry.size)}
                </p>
              </div>
              <div className="flex gap-1">
                <button
                  onClick={() => handleResume(entry)}
                  disabled={resumeLoading === entry.filename}
                  title="Resume conversation"
                  className="p-1.5 text-gray-400 hover:text-purple-600 hover:bg-purple-50 rounded transition-colors disabled:opacity-50"
                >
                  {resumeLoading === entry.filename ? <Spinner /> : <PlayIcon />}
                </button>
                <button
                  onClick={() => onDelete(entry.filename)}
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
