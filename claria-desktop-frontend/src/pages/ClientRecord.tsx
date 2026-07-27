import { useState, useEffect, useCallback, useRef } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  listRecordFiles,
  searchRecordContents,
  uploadRecordFile,
  deleteRecordFile,
  getRecordFileText,
  createTextRecordFile,
  updateTextRecordFile,
  loadChatHistory,
  listFileVersions,
  getFileVersionText,
  restoreFileVersion,
  listDeletedFiles,
  restoreDeletedFile,
  loadConfig,
  type RecordFile,
  type ChatHistoryDetail,
  type ChatModel,
  type DeletedFile,
  type TranscriptionPreferences,
} from "../lib/tauri";
import TranscribeWizard from "../components/TranscribeWizard";
import MemoRecorderBar from "../components/MemoRecorderBar";
import MemoReviewModal from "../components/MemoReviewModal";
import VersionHistoryModal from "../components/VersionHistoryModal";
import TranscriptEditor from "../components/TranscriptEditor";
import Spinner from "../components/Spinner";
import MoreToggle from "../components/MoreToggle";
import DeletedSection from "../components/DeletedSection";
import { ErrorBanner } from "../components/StateCards";
import Modal from "../components/Modal";
import {
  BackButton,
  CloseIcon,
  FolderIcon,
  PlayIcon,
  SearchIcon,
  TrashIcon,
} from "../components/icons";
import { formatDateTime, formatFileSize } from "../lib/format";
import { searchMatches } from "../lib/search";
import {
  AUDIO_EXTENSIONS,
  CHAT_HISTORY_PREFIX,
  isAudioSidecar,
  versionKeyFor,
} from "../lib/recordFiles";
import { transcribeSummary, transcribeTooltip } from "../lib/transcribe";
import { useMemoRecorder } from "../lib/useMemoRecorder";
import { useMoreMode } from "../lib/useMoreMode";
import ClientChat from "./ClientChat";
import type { Page } from "../App";
import type { ResumeChat } from "./ClientChat";

type Tab = "record" | "chat";

export default function ClientRecord({
  navigate,
  clientId,
  clientName,
  chatModels,
  chatModelsLoading,
  chatModelsError,
  preferredModelId,
}: {
  navigate: (page: Page) => void;
  clientId: string;
  clientName: string;
  chatModels: ChatModel[];
  chatModelsLoading: boolean;
  chatModelsError: string | null;
  preferredModelId?: string | null;
}) {
  const [tab, setTab] = useState<Tab>("record");
  const [resumeChat, setResumeChat] = useState<ResumeChat | null>(null);

  function handleResumeChat(detail: ChatHistoryDetail) {
    setResumeChat({
      chatId: detail.chat_id,
      modelId: detail.model_id,
      messages: detail.messages.map((m) => ({
        role: m.role,
        content: m.content,
      })),
      usageByIndex: detail.messages.map((m) => m.usage),
      lastActivityIso: detail.created_at,
    });
    setTab("chat");
  }

  return (
    <div className="flex flex-col h-screen">
      {/* Header */}
      <div className="flex items-center gap-3 px-6 py-4 border-b border-gray-200 bg-white">
        <BackButton onClick={() => navigate("clients")} />
        <h2 className="text-lg font-semibold flex-1">{clientName}</h2>

        {/* Tabs */}
        <div className="flex border border-gray-200 rounded-lg overflow-hidden">
          <button
            data-tab="record"
            onClick={() => setTab("record")}
            className={`px-4 py-1.5 text-sm font-medium transition-colors ${
              tab === "record"
                ? "bg-blue-600 text-white"
                : "bg-white text-gray-600 hover:bg-gray-50"
            }`}
          >
            Record
          </button>
          <button
            data-tab="chat"
            onClick={() => setTab("chat")}
            className={`px-4 py-1.5 text-sm font-medium transition-colors ${
              tab === "chat"
                ? "bg-blue-600 text-white"
                : "bg-white text-gray-600 hover:bg-gray-50"
            }`}
          >
            Chat
          </button>
        </div>
      </div>

      {/* Tab content */}
      {tab === "record" ? (
        <RecordTab clientId={clientId} onResumeChat={handleResumeChat} />
      ) : (
        <ClientChat
          navigate={navigate}
          clientId={clientId}
          clientName={clientName}
          embedded
          resumeChat={resumeChat}
          onResumeChatConsumed={() => setResumeChat(null)}
          chatModels={chatModels}
          chatModelsLoading={chatModelsLoading}
          chatModelsError={chatModelsError}
          preferredModelId={preferredModelId}
        />
      )}
    </div>
  );
}

function RecordTab({ clientId, onResumeChat }: { clientId: string; onResumeChat: (detail: ChatHistoryDetail) => void }) {
  const [files, setFiles] = useState<RecordFile[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [uploading, setUploading] = useState<string[]>([]);
  const [previewText, setPreviewText] = useState<string | null>(null);
  const [previewFilename, setPreviewFilename] = useState<string | null>(null);
  const [editText, setEditText] = useState<string | null>(null);
  const [editFilename, setEditFilename] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState<string | null>(null);
  const [showCreateText, setShowCreateText] = useState(false);
  const [createFilename, setCreateFilename] = useState("");
  const [createContent, setCreateContent] = useState("");
  const [creating, setCreating] = useState(false);
  const [chatFolderOpen, setChatFolderOpen] = useState(false);
  const [resumeLoading, setResumeLoading] = useState<string | null>(null);

  // Transcription wizard state + cached prefs (used by the drag-zone tooltip).
  const [showTranscribeWizard, setShowTranscribeWizard] = useState(false);
  const [wizardDroppedFile, setWizardDroppedFile] = useState<string | null>(null);
  const [transcriptionPrefs, setTranscriptionPrefs] =
    useState<TranscriptionPreferences | null>(null);
  // Tauri's drag-drop fires at the webview level, not DOM level, so it doesn't
  // respect modal stacking. We mirror `showTranscribeWizard` into a ref so the
  // listener closure can read the *current* value without re-registering on
  // every open/close — and when the wizard is open, drops route into the
  // wizard instead of starting a legacy upload.
  const wizardOpenRef = useRef(false);
  useEffect(() => {
    wizardOpenRef.current = showTranscribeWizard;
  }, [showTranscribeWizard]);

  // More mode state
  const {
    moreMode,
    toggleMoreMode,
    deletedItems: deletedFiles,
    deletedLoading: deletedFilesLoading,
    restoringKey,
    restore,
  } = useMoreMode<DeletedFile>(
    () => listDeletedFiles(clientId),
    (e) => setError(String(e)),
  );

  // Which file's version history is open, if any.
  const [versionFile, setVersionFile] = useState<string | null>(null);

  // Live memo recording — microphone, PCM buffer and timers all live in here.
  const memo = useMemoRecorder({
    clientId,
    onError: setError,
    onSaved: () => refresh(),
  });

  // Search matches filenames instantly (case-insensitive substring); the
  // debounced query is also matched against each file's extracted text on
  // the backend, and files that only match by content get a badge.
  const [search, setSearch] = useState("");
  // The search input replaces the header action buttons while open; closing
  // clears the query so files are never filtered by a hidden search box.
  const [searchOpen, setSearchOpen] = useState(false);
  const [debouncedSearch, setDebouncedSearch] = useState("");
  useEffect(() => {
    const t = setTimeout(() => setDebouncedSearch(search.trim()), 300);
    return () => clearTimeout(t);
  }, [search]);

  const [contentMatches, setContentMatches] = useState<Set<string>>(
    new Set(),
  );
  useEffect(() => {
    if (!debouncedSearch) {
      setContentMatches(new Set());
      return;
    }
    let stale = false;
    searchRecordContents(clientId, debouncedSearch)
      .then((names) => {
        if (!stale) setContentMatches(new Set(names));
      })
      .catch((e) => {
        if (!stale) setError(String(e));
      });
    return () => {
      stale = true;
    };
  }, [clientId, debouncedSearch]);

  const query = search.trim();
  const chatHistoryFiles = files
    .filter((f) => f.filename.startsWith(CHAT_HISTORY_PREFIX))
    .sort((a, b) => (b.uploaded_at ?? "").localeCompare(a.uploaded_at ?? ""));
  const regularFiles = files.filter(
    (f) =>
      !f.filename.startsWith(CHAT_HISTORY_PREFIX) &&
      (searchMatches(f.filename, query) || contentMatches.has(f.filename)),
  );

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const result = await listRecordFiles(clientId);
      setFiles(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [clientId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Load transcription preferences once for the drag-zone tooltip. Cheap;
  // no need to re-run on tab switches.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const info = await loadConfig();
        if (!cancelled) setTranscriptionPrefs(info.transcription);
      } catch {
        // Silent — the tooltip just won't show prefs in that case.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Tauri drag-and-drop event listener.
  //
  // Two subtleties:
  //
  // 1. **StrictMode race.** `onDragDropEvent` returns a Promise that resolves
  //    to the unlisten function. In React 18 StrictMode, effects mount → clean
  //    up → mount again. The first cleanup runs before the Promise resolves,
  //    so the original `unlisten?.()` was a no-op and both mounts ended up
  //    with live listeners — the bug the user observed as "two uploads at
  //    once". `cancelled` flag drains the second listener if cleanup runs
  //    before registration completes.
  //
  // 2. **Wizard modal stacking.** Tauri's drag-drop is webview-level, not
  //    DOM-level — dropping on the wizard modal still fires this handler and
  //    would upload via the legacy fast path, bypassing the wizard's options.
  //    Bail out via `wizardOpenRef` when the modal is open; the user has the
  //    "Choose a file…" picker inside the wizard.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (
          event.payload.type === "enter" ||
          event.payload.type === "over"
        ) {
          setDragging(true);
        } else if (event.payload.type === "leave") {
          setDragging(false);
        } else if (event.payload.type === "drop") {
          setDragging(false);
          if (wizardOpenRef.current) {
            // Route the drop into the open wizard — it adopts the path as
            // the chosen file. We forward the first path only; the wizard
            // is single-file.
            const first = event.payload.paths[0];
            if (first) setWizardDroppedFile(first);
            return;
          }
          handleFileDrop(event.payload.paths);
        }
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err) => {
        console.error("Failed to register drag-drop listener:", err);
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clientId]);

  async function handleFileDrop(paths: string[]) {
    for (const path of paths) {
      const filename = path.split("/").pop() ?? path;
      setUploading((prev) => [...prev, filename]);
      try {
        await uploadRecordFile(clientId, path);
      } catch (e) {
        setError(String(e));
      } finally {
        setUploading((prev) => prev.filter((f) => f !== filename));
      }
    }
    await refresh();
  }

  async function handlePreview(filename: string) {
    setPreviewFilename(filename);
    try {
      const text = await getRecordFileText(clientId, filename);
      setPreviewText(text);
    } catch (e) {
      setPreviewText(`Error loading preview: ${String(e)}`);
    }
  }

  function handleClosePreview() {
    setPreviewText(null);
    setPreviewFilename(null);
  }

  function handleCancelEdit() {
    setEditText(null);
    setEditFilename(null);
  }

  function handleCancelCreateText() {
    setShowCreateText(false);
    setCreateFilename("");
    setCreateContent("");
  }

  async function handleEdit(filename: string) {
    setEditFilename(filename);
    try {
      const text = await getRecordFileText(clientId, filename);
      setEditText(text);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleSaveEdit() {
    if (!editFilename || editText === null) return;
    setSaving(true);
    setError(null);
    try {
      await updateTextRecordFile(clientId, editFilename, editText);
      setEditFilename(null);
      setEditText(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete(filename: string) {
    setDeleteConfirm(null);
    try {
      await deleteRecordFile(clientId, filename);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleResume(filename: string) {
    // Extract UUID from "chat-history/{uuid}.json"
    const chatId = filename.replace(CHAT_HISTORY_PREFIX, "").replace(".json", "");
    setResumeLoading(filename);
    try {
      const detail = await loadChatHistory(clientId, chatId);
      onResumeChat(detail);
    } catch (e) {
      setError(String(e));
    } finally {
      setResumeLoading(null);
    }
  }

  async function handleCreateTextFile() {
    if (!createFilename.trim()) return;
    setCreating(true);
    setError(null);
    try {
      await createTextRecordFile(clientId, createFilename.trim(), createContent);
      setShowCreateText(false);
      setCreateFilename("");
      setCreateContent("");
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  }

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-2xl mx-auto p-8">
        {/* Error */}
        {error && <ErrorBanner message={error} />}

        {/* File list / drop zone */}
        <div
          className={`border-2 rounded-lg transition-colors ${
            dragging
              ? "border-blue-400 bg-blue-50"
              : "border-gray-200 bg-white"
          }`}
        >
          <div className="px-4 py-2 border-b border-gray-100 bg-gray-50 rounded-t-lg flex items-center justify-between">
            <h3 className="text-sm font-semibold text-gray-700">Files</h3>
            <div className="flex gap-2">
              {searchOpen && (
                <>
                  <input
                    type="search"
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Escape") {
                        setSearch("");
                        setSearchOpen(false);
                      }
                    }}
                    placeholder="Search file names and contents…"
                    title="Show files whose name or contents contain this text"
                    autoFocus
                    className="w-72 px-2 py-1 text-xs border border-gray-300 rounded focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500"
                  />
                  <button
                    onClick={() => {
                      setSearch("");
                      setSearchOpen(false);
                    }}
                    title="Close search"
                    className="p-1.5 rounded border border-gray-300 text-gray-400 hover:bg-gray-100 transition-colors"
                  >
                    <CloseIcon className="w-3.5 h-3.5" />
                  </button>
                </>
              )}
              {!searchOpen && (
                <button
                  onClick={() => setSearchOpen(true)}
                  title="Search file names and contents"
                  className="p-1.5 rounded border border-gray-300 text-gray-400 hover:bg-gray-100 transition-colors"
                >
                  <SearchIcon />
                </button>
              )}
              {!searchOpen && (
                <MoreToggle
                  active={moreMode}
                  onClick={toggleMoreMode}
                  title={moreMode ? "Hide version history" : "Show version history"}
                />
              )}
              {!searchOpen && memo.ready && memo.state === "idle" && (
                <button
                  onClick={memo.start}
                  className="px-3 py-1 text-xs font-medium text-white bg-red-600 rounded hover:bg-red-700 transition-colors flex items-center gap-1"
                >
                  <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24">
                    <path d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3z" />
                    <path d="M17 11c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-3.08c3.39-.49 6-3.39 6-6.92h-2z" />
                  </svg>
                  Record Memo
                </button>
              )}
              {!searchOpen && (
                <button
                  onClick={() => setShowTranscribeWizard(true)}
                  className="px-3 py-1 text-xs font-medium text-white bg-blue-600 rounded hover:bg-blue-700 transition-colors"
                  title="Pick an audio file and configure transcription per-file"
                >
                  Upload Audio File…
                </button>
              )}
              {!searchOpen && (
                <button
                  onClick={() => setShowCreateText(true)}
                  className="px-3 py-1 text-xs font-medium text-white bg-green-600 rounded hover:bg-green-700 transition-colors"
                >
                  Create Text File
                </button>
              )}
            </div>
          </div>

          <MemoRecorderBar
            state={memo.state}
            elapsed={memo.elapsed}
            transcript={memo.transcript}
            onTranscriptChange={memo.setTranscript}
            multilingual={memo.multilingual}
            detectedLanguage={memo.detectedLanguage}
            gpu={memo.gpu}
            modelLabel={memo.modelLabel}
            onPause={memo.pause}
            onResume={memo.resume}
            onDone={memo.done}
            onCancel={memo.cancel}
          />

          {/* Loading */}
          {loading && (
            <div className="p-8 text-center">
              <div className="flex items-center justify-center gap-2 text-gray-500 text-sm">
                <Spinner />
                <span>Loading files...</span>
              </div>
            </div>
          )}

          {/* Chat history folder */}
          {!loading && chatHistoryFiles.length > 0 && (
            <div className="border-b border-gray-100">
              <button
                onClick={() => setChatFolderOpen(!chatFolderOpen)}
                className="w-full px-4 py-3 flex items-center gap-3 hover:bg-gray-50 transition-colors"
              >
                <div className="w-8 h-8 rounded flex items-center justify-center bg-purple-100 text-purple-600">
                  <FolderIcon />
                </div>
                <div className="flex-1 min-w-0 text-left">
                  <p className="text-sm font-medium text-gray-900">Chat History</p>
                  <p className="text-xs text-gray-400">{chatHistoryFiles.length} conversation{chatHistoryFiles.length !== 1 ? "s" : ""}</p>
                </div>
                <svg className={`w-4 h-4 text-gray-400 transition-transform ${chatFolderOpen ? "rotate-90" : ""}`} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                </svg>
              </button>
              {chatFolderOpen && (
                <div className="divide-y divide-gray-100 bg-gray-50/50">
                  {chatHistoryFiles.map((file) => {
                    const displayName = file.filename.replace(CHAT_HISTORY_PREFIX, "").replace(".json", "");
                    const shortId = displayName.length > 8 ? displayName.slice(0, 8) + "..." : displayName;
                    return (
                      <div
                        key={file.filename}
                        className="px-4 py-3 pl-8 flex items-center gap-3"
                      >
                        <div className="w-8 h-8 rounded flex items-center justify-center bg-purple-50 text-purple-500 text-xs font-bold">
                          AI
                        </div>
                        <div className="flex-1 min-w-0">
                          <p className="text-sm font-medium text-gray-900 truncate">
                            {shortId}
                          </p>
                          <p className="text-xs text-gray-400">
                            {formatFileSize(file.size)}
                          </p>
                        </div>
                        <div className="flex gap-1">
                          <button
                            onClick={() => handleResume(file.filename)}
                            disabled={resumeLoading === file.filename}
                            title="Resume conversation"
                            className="p-1.5 text-gray-400 hover:text-purple-600 hover:bg-purple-50 rounded transition-colors disabled:opacity-50"
                          >
                            {resumeLoading === file.filename ? (
                              <Spinner />
                            ) : (
                              <PlayIcon />
                            )}
                          </button>
                          <button
                            onClick={() => setDeleteConfirm(file.filename)}
                            title="Delete chat history"
                            className="p-1.5 text-gray-400 hover:text-red-600 hover:bg-red-50 rounded transition-colors"
                          >
                            <TrashIcon />
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {/* Regular file rows */}
          {!loading && regularFiles.length > 0 && (
            <div className="divide-y divide-gray-100">
              {regularFiles.map((file) => (
                <div
                  key={file.filename}
                  className="px-4 py-3 flex items-center gap-3"
                >
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
                    <p className="text-xs text-gray-400">
                      {formatFileSize(file.size)}
                    </p>
                  </div>
                  <div className="flex gap-1">
                    {moreMode && (
                      <button
                        onClick={() => setVersionFile(file.filename)}
                        title="Version history"
                        className="p-1.5 text-gray-400 hover:text-blue-600 hover:bg-blue-50 rounded transition-colors"
                      >
                        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
                        </svg>
                      </button>
                    )}
                    {file.filename.endsWith(".txt") ? (
                      <button
                        onClick={() => handleEdit(file.filename)}
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
                        onClick={() => handlePreview(file.filename)}
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
                      onClick={() => setDeleteConfirm(file.filename)}
                      title="Delete file"
                      className="p-1.5 text-gray-400 hover:text-red-600 hover:bg-red-50 rounded transition-colors"
                    >
                      <TrashIcon />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}

          {/* Uploading indicator */}
          {uploading.length > 0 && (
            <div className="divide-y divide-gray-100 border-t border-gray-100">
              {uploading.map((filename) => (
                <div
                  key={filename}
                  className="px-4 py-3 flex items-center gap-3"
                >
                  <Spinner />
                  <div className="flex-1 min-w-0">
                    <p className="text-sm text-gray-500 truncate">
                      Uploading {filename}...
                    </p>
                  </div>
                </div>
              ))}
            </div>
          )}

          {/* No search matches */}
          {!loading && search.trim() !== "" && regularFiles.length === 0 && (
            <div className="px-4 py-6 text-center">
              <p className="text-sm text-gray-400">
                No file names or contents match &ldquo;{search.trim()}&rdquo;
              </p>
            </div>
          )}

          {/* Drop hint */}
          {!loading && (
            <div
              className={`px-4 py-6 text-center ${
                files.length === 0 && uploading.length === 0 ? "py-12" : ""
              }`}
              title={transcribeTooltip(transcriptionPrefs)}
            >
              <p
                className={`text-sm ${
                  dragging ? "text-blue-600 font-medium" : "text-gray-400"
                }`}
              >
                {dragging
                  ? "Drop files to upload"
                  : "Drag files here \u2014 PDF, DOCX, audio, or text"}
              </p>
              {!dragging && transcriptionPrefs && (
                <p className="text-xs text-gray-400 mt-1">
                  {transcribeSummary(transcriptionPrefs)}
                </p>
              )}
            </div>
          )}
        </div>
      </div>

      {/* Preview / segment editor.
          Audio sidecars get the structured TranscriptEditor (header parser,
          per-segment rows, speaker rename, save / restore-to-original via S3
          versioning). PDF/DOCX sidecars are plain text — render read-only. */}
      {previewText !== null &&
        previewFilename &&
        (isAudioSidecar(previewFilename) ? (
          <TranscriptEditor
            clientId={clientId}
            filename={previewFilename}
            initialBody={previewText}
            onClose={handleClosePreview}
            onSaved={(newBody) => setPreviewText(newBody)}
          />
        ) : (
          <Modal
            open
            onClose={handleClosePreview}
            title={previewFilename}
            className="max-w-2xl p-6 max-h-[80vh] flex flex-col"
          >
            <div className="flex-1 overflow-y-auto border border-gray-200 rounded-lg p-4">
              <pre className="text-sm text-gray-700 whitespace-pre-wrap font-mono">
                {previewText}
              </pre>
            </div>
            <div className="flex justify-end mt-4">
              <button
                onClick={handleClosePreview}
                className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
              >
                Close
              </button>
            </div>
          </Modal>
        ))}

      {/* Edit text file modal */}
      {editText !== null && editFilename && (
        <Modal
          open
          onClose={handleCancelEdit}
          title={editFilename}
          className="max-w-2xl p-6 max-h-[80vh] flex flex-col"
          showClose={false}
          dismissible={!saving}
        >
          <textarea
            value={editText}
            onChange={(e) => setEditText(e.target.value)}
            disabled={saving}
            className="flex-1 min-h-[300px] w-full px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono resize-none focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:bg-gray-50"
          />
          <div className="flex justify-end gap-3 mt-4">
            <button
              onClick={handleCancelEdit}
              disabled={saving}
              className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800 disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              onClick={handleSaveEdit}
              disabled={saving}
              className="px-4 py-2 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
            >
              {saving ? "Saving..." : "Save"}
            </button>
          </div>
        </Modal>
      )}

      {/* Delete confirmation */}
      {deleteConfirm && (
        <Modal
          open
          onClose={() => setDeleteConfirm(null)}
          title="Delete file?"
          className="max-w-sm p-6"
          showClose={false}
        >
          <p className="text-sm text-gray-600 mb-6">
            Delete <span className="font-medium">{deleteConfirm}</span> and its
            extracted text? You can restore this file again later if necessary.
          </p>
          <div className="flex justify-end gap-3">
            <button
              onClick={() => setDeleteConfirm(null)}
              className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
            >
              Cancel
            </button>
            <button
              onClick={() => handleDelete(deleteConfirm)}
              className="px-4 py-2 text-sm text-white bg-red-600 rounded-lg hover:bg-red-700 transition-colors"
            >
              Delete
            </button>
          </div>
        </Modal>
      )}

      {/* Transcription wizard */}
      {showTranscribeWizard && (
        <TranscribeWizard
          clientId={clientId}
          droppedFilePath={wizardDroppedFile}
          isDragging={dragging}
          onClose={() => {
            setShowTranscribeWizard(false);
            setWizardDroppedFile(null);
          }}
          onUploaded={() => {
            void refresh();
          }}
        />
      )}

      {/* Create text file modal */}
      {showCreateText && (
        <Modal
          open
          onClose={handleCancelCreateText}
          title="Create Text File"
          className="max-w-2xl p-6 max-h-[80vh] flex flex-col"
          showClose={false}
          dismissible={!creating}
        >
          <input
            type="text"
            placeholder="Filename (e.g. intake-notes)"
            value={createFilename}
            onChange={(e) => setCreateFilename(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-green-500 focus:border-transparent mb-3"
            autoFocus
          />
          <textarea
            placeholder="File content..."
            value={createContent}
            onChange={(e) => setCreateContent(e.target.value)}
            className="flex-1 min-h-[200px] w-full px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono resize-none focus:outline-none focus:ring-2 focus:ring-green-500 focus:border-transparent mb-4"
          />
          <div className="flex justify-end gap-3">
            <button
              onClick={handleCancelCreateText}
              className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
              disabled={creating}
            >
              Cancel
            </button>
            <button
              onClick={handleCreateTextFile}
              disabled={creating || !createFilename.trim()}
              className="px-4 py-2 text-sm text-white bg-green-600 rounded-lg hover:bg-green-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {creating ? "Creating..." : "Create"}
            </button>
          </div>
        </Modal>
      )}

      {memo.state === "review" && (
        <MemoReviewModal
          filename={memo.filename}
          onFilenameChange={memo.setFilename}
          transcript={memo.transcript}
          onTranscriptChange={memo.setTranscript}
          saving={memo.saving}
          onDiscard={memo.cancel}
          onSave={memo.save}
        />
      )}

      {/* Deleted files (More mode) */}
      {moreMode && !loading && (
        <div className="max-w-2xl mx-auto px-8 pb-8">
          <DeletedSection
            title="Deleted Files"
            noun="files"
            loading={deletedFilesLoading}
            items={deletedFiles}
            itemKey={(df) => df.filename}
            primary={(df) => df.filename}
            subtitle={(df) =>
              df.deleted_at ? `Deleted ${formatDateTime(df.deleted_at)}` : "Deleted"
            }
            icon={(df) => <FileIcon filename={df.filename} />}
            restoringKey={restoringKey}
            onRestore={(df) =>
              restore(
                df.filename,
                () => restoreDeletedFile(clientId, df.filename, df.version_id),
                (f) => f.filename === df.filename,
                refresh,
              )
            }
          />
        </div>
      )}

      {versionFile && (
        <VersionHistoryModal
          title={
            isAudioSidecar(versionFile)
              ? `Transcript History: ${versionFile}`
              : `Version History: ${versionFile}`
          }
          source={{
            list: () => listFileVersions(clientId, versionKeyFor(versionFile)),
            getText: (versionId) =>
              getFileVersionText(clientId, versionKeyFor(versionFile), versionId),
            restore: (versionId) =>
              restoreFileVersion(clientId, versionKeyFor(versionFile), versionId),
          }}
          onClose={() => setVersionFile(null)}
          onRestored={refresh}
          onError={setError}
          enableCompare
          showFooterClose
          className="max-w-4xl p-6 max-h-[90vh] flex flex-col"
        />
      )}
    </div>
  );
}

function FileIcon({ filename }: { filename: string }) {
  const ext = filename.split(".").pop()?.toLowerCase() ?? "";
  const isPdf = ext === "pdf";
  const isDoc = ext === "docx" || ext === "doc";
  const isAudio = AUDIO_EXTENSIONS.has(ext);

  return (
    <div
      className={`w-8 h-8 rounded flex items-center justify-center text-xs font-bold ${
        isPdf
          ? "bg-red-100 text-red-600"
          : isDoc
            ? "bg-blue-100 text-blue-600"
            : isAudio
              ? "bg-purple-100 text-purple-600"
              : "bg-gray-100 text-gray-500"
      }`}
    >
      {isPdf ? "PDF" : isDoc ? "DOC" : isAudio ? "AUD" : ext.toUpperCase().slice(0, 3) || "?"}
    </div>
  );
}

