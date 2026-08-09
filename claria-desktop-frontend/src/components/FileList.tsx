import FileIcon from "./FileIcon";
import { TrashIcon } from "./icons";
import { formatFileSize } from "../lib/format";
import { searchMatches } from "../lib/search";
import type { RecordFile } from "../lib/tauri";

/**
 * The record's regular files, one row each.
 *
 * `.txt` files open the editor and everything else opens the read-only text
 * preview. Uploaded structured text keeps its original format, while generated
 * document and transcript sidecars remain derived output that must not be
 * edited independently of the source file.
 */
export default function FileList({
  files,
  query,
  contentMatches,
  showVersions,
  onOpenVersions,
  onEdit,
  onPreview,
  onDelete,
}: {
  files: RecordFile[];
  /** Current search text, used to explain why a row matched. */
  query: string;
  /** Filenames whose *contents* matched the search. */
  contentMatches: ReadonlySet<string>;
  /** Whether the per-row version history button is offered. */
  showVersions: boolean;
  onOpenVersions: (filename: string) => void;
  onEdit: (filename: string) => void;
  onPreview: (filename: string) => void;
  onDelete: (filename: string) => void;
}) {
  return (
    <div className="divide-y divide-gray-100">
      {files.map((file) => (
        <div key={file.filename} className="px-4 py-3 flex items-center gap-3">
          <FileIcon filename={file.filename} />
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 min-w-0">
              <p className="text-sm font-medium text-gray-900 truncate">
                {file.filename}
              </p>
              {query !== "" &&
                contentMatches.has(file.filename) &&
                !searchMatches(file.filename, query) && (
                  <span className="shrink-0 px-1.5 py-0.5 text-[10px] font-medium text-blue-700 bg-blue-50 border border-blue-200 rounded">
                    match in contents
                  </span>
                )}
            </div>
            <p className="text-xs text-gray-400">{formatFileSize(file.size)}</p>
          </div>
          <div className="flex gap-1">
            {showVersions && (
              <button
                onClick={() => onOpenVersions(file.filename)}
                title="Version history"
                className="p-1.5 text-gray-400 hover:text-blue-600 hover:bg-blue-50 rounded transition-colors"
              >
                <svg
                  className="w-4 h-4"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
                  />
                </svg>
              </button>
            )}
            {file.filename.endsWith(".txt") ? (
              <button
                onClick={() => onEdit(file.filename)}
                title="Edit file"
                className="p-1.5 text-gray-400 hover:text-blue-600 hover:bg-blue-50 rounded transition-colors"
              >
                <svg
                  className="w-4 h-4"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
                  />
                </svg>
              </button>
            ) : (
              <button
                onClick={() => onPreview(file.filename)}
                title="Preview text"
                className="p-1.5 text-gray-400 hover:text-blue-600 hover:bg-blue-50 rounded transition-colors"
              >
                <svg
                  className="w-4 h-4"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                  />
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"
                  />
                </svg>
              </button>
            )}
            <button
              onClick={() => onDelete(file.filename)}
              title="Delete file"
              className="p-1.5 text-gray-400 hover:text-red-600 hover:bg-red-50 rounded transition-colors"
            >
              <TrashIcon />
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}
