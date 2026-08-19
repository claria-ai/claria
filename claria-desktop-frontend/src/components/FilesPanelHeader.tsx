import MoreToggle from "./MoreToggle";
import { CloseIcon, SearchIcon } from "./icons";

/**
 * Title row of the file panel: search box, and the action buttons it hides.
 *
 * Opening search replaces the actions rather than sitting beside them, and
 * closing it clears the query — a filtered list with no visible search box is
 * indistinguishable from an empty record.
 */
export default function FilesPanelHeader({
  search,
  onSearchChange,
  searchOpen,
  onOpenSearch,
  onCloseSearch,
  moreMode,
  onToggleMoreMode,
  showRecordMemo,
  onRecordMemo,
  onUploadAudio,
  onCreateTextFile,
}: {
  search: string;
  onSearchChange: (value: string) => void;
  searchOpen: boolean;
  onOpenSearch: () => void;
  /** Closes the search box and clears the query. */
  onCloseSearch: () => void;
  moreMode: boolean;
  onToggleMoreMode: () => void;
  /** A local speech model is installed and no recording is in progress. */
  showRecordMemo: boolean;
  onRecordMemo: () => void;
  onUploadAudio: () => void;
  onCreateTextFile: () => void;
}) {
  return (
    <div className="px-4 py-2 border-b border-gray-100 bg-gray-50 rounded-t-lg flex items-center justify-between">
      <h3 className="text-sm font-semibold text-gray-700">Files</h3>
      <div className="flex gap-2">
        {searchOpen ? (
          <>
            <input
              type="search"
              value={search}
              onChange={(e) => onSearchChange(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") onCloseSearch();
              }}
              placeholder="Search file names and contents…"
              title="Show files whose name or contents contain this text"
              autoFocus
              className="w-72 px-2 py-1 text-xs border border-gray-300 rounded focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500"
            />
            <button
              onClick={onCloseSearch}
              title="Close search"
              className="p-1.5 rounded border border-gray-300 text-gray-400 hover:bg-gray-100 transition-colors"
            >
              <CloseIcon className="w-3.5 h-3.5" />
            </button>
          </>
        ) : (
          <>
            <button
              onClick={onOpenSearch}
              title="Search file names and contents"
              className="p-1.5 rounded border border-gray-300 text-gray-400 hover:bg-gray-100 transition-colors"
            >
              <SearchIcon />
            </button>
            <MoreToggle
              active={moreMode}
              onClick={onToggleMoreMode}
              title={moreMode ? "Hide version history" : "Show version history"}
            />
            {showRecordMemo && (
              <button
                onClick={onRecordMemo}
                className="px-3 py-1 text-xs font-medium text-white bg-red-600 rounded hover:bg-red-700 transition-colors flex items-center gap-1"
              >
                <svg
                  className="w-3 h-3"
                  fill="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3z" />
                  <path d="M17 11c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-3.08c3.39-.49 6-3.39 6-6.92h-2z" />
                </svg>
                Record Memo
              </button>
            )}
            <button
              onClick={onUploadAudio}
              className="px-3 py-1 text-xs font-medium text-white bg-blue-600 rounded hover:bg-blue-700 transition-colors"
              title="Pick an audio file and configure transcription per-file"
            >
              Upload Audio File…
            </button>
            <button
              onClick={onCreateTextFile}
              className="px-3 py-1 text-xs font-medium text-white bg-green-600 rounded hover:bg-green-700 transition-colors"
            >
              Create Text File
            </button>
          </>
        )}
      </div>
    </div>
  );
}
