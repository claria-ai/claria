import { useState, useEffect, useCallback } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  listRecordFiles,
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
  type RecordFile,
  type ChatHistoryDetail,
  type ChatModel,
  type FileVersion,
  type DeletedFile,
} from "../lib/tauri";
import { diffLines, type DiffLine } from "../lib/diff";
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
}: {
  navigate: (page: Page) => void;
  clientId: string;
  clientName: string;
  chatModels: ChatModel[];
  chatModelsLoading: boolean;
  chatModelsError: string | null;
}) {
  const [tab, setTab] = useState<Tab>("record");
  const [resumeChat, setResumeChat] = useState<ResumeChat | null>(null);

  function handleResumeChat(detail: ChatHistoryDetail) {
    setResumeChat({
      chatId: detail.chat_id,
      modelId: detail.model_id,
      messages: detail.messages,
    });
    setTab("chat");
  }

  return (
    <div className="flex flex-col h-screen">
      {/* Header */}
      <div className="flex items-center gap-3 px-6 py-4 border-b border-gray-200 bg-white">
        <button
          onClick={() => navigate("clients")}
          className="text-gray-500 hover:text-gray-700 transition-colors"
        >
          <svg
            className="w-5 h-5"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M15 19l-7-7 7-7"
            />
          </svg>
        </button>
        <h2 className="text-lg font-semibold flex-1">{clientName}</h2>

        {/* Tabs */}
        <div className="flex border border-gray-200 rounded-lg overflow-hidden">
          <button
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

  // More mode state
  const [moreMode, setMoreMode] = useState(false);
  const [deletedFiles, setDeletedFiles] = useState<DeletedFile[]>([]);
  const [deletedFilesLoading, setDeletedFilesLoading] = useState(false);
  const [restoringDeletedFile, setRestoringDeletedFile] = useState<string | null>(null);

  // Version history modal state
  const [versionFile, setVersionFile] = useState<string | null>(null);
  const [versions, setVersions] = useState<FileVersion[]>([]);
  const [versionsLoading, setVersionsLoading] = useState(false);
  const [versionPreview, setVersionPreview] = useState<{ versionId: string; text: string } | null>(null);
  const [versionPreviewLoading, setVersionPreviewLoading] = useState(false);
  const [selectedVersions, setSelectedVersions] = useState<Set<string>>(new Set());
  const [diffResult, setDiffResult] = useState<DiffLine[] | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [restoringVersion, setRestoringVersion] = useState(false);

  const CHAT_HISTORY_PREFIX = "chat-history/";

  const chatHistoryFiles = files
    .filter((f) => f.filename.startsWith(CHAT_HISTORY_PREFIX))
    .sort((a, b) => (b.uploaded_at ?? "").localeCompare(a.uploaded_at ?? ""));
  const regularFiles = files.filter((f) => !f.filename.startsWith(CHAT_HISTORY_PREFIX));

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

  // Tauri drag-and-drop event listener.
  useEffect(() => {
    let unlisten: (() => void) | null = null;

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
          handleFileDrop(event.payload.paths);
        }
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((err) => {
        console.error("Failed to register drag-drop listener:", err);
      });

    return () => {
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

  async function handleToggleMore() {
    const next = !moreMode;
    setMoreMode(next);
    if (next) {
      setDeletedFilesLoading(true);
      try {
        setDeletedFiles(await listDeletedFiles(clientId));
      } catch (e) {
        setError(String(e));
      } finally {
        setDeletedFilesLoading(false);
      }
    }
  }

  async function handleOpenVersions(filename: string) {
    setVersionFile(filename);
    setVersionsLoading(true);
    setVersionPreview(null);
    setSelectedVersions(new Set());
    setDiffResult(null);
    try {
      setVersions(await listFileVersions(clientId, filename));
    } catch (e) {
      setError(String(e));
    } finally {
      setVersionsLoading(false);
    }
  }

  function handleCloseVersions() {
    setVersionFile(null);
    setVersions([]);
    setVersionPreview(null);
    setSelectedVersions(new Set());
    setDiffResult(null);
  }

  async function handleViewVersion(versionId: string) {
    if (versionPreview?.versionId === versionId) {
      setVersionPreview(null);
      return;
    }
    setVersionPreviewLoading(true);
    try {
      const text = await getFileVersionText(clientId, versionFile!, versionId);
      setVersionPreview({ versionId, text });
    } catch (e) {
      setVersionPreview({ versionId, text: `Error: ${String(e)}` });
    } finally {
      setVersionPreviewLoading(false);
    }
  }

  function handleToggleVersionSelect(versionId: string) {
    setSelectedVersions((prev) => {
      const next = new Set(prev);
      if (next.has(versionId)) {
        next.delete(versionId);
      } else {
        if (next.size >= 2) {
          // Replace the oldest selection
          const [first] = next;
          next.delete(first);
        }
        next.add(versionId);
      }
      return next;
    });
    setDiffResult(null);
  }

  async function handleCompare() {
    if (selectedVersions.size !== 2 || !versionFile) return;
    setDiffLoading(true);
    setDiffResult(null);
    try {
      const [v1, v2] = [...selectedVersions];
      const [text1, text2] = await Promise.all([
        getFileVersionText(clientId, versionFile, v1),
        getFileVersionText(clientId, versionFile, v2),
      ]);
      // Order by version position: v1 is older, v2 is newer
      const idx1 = versions.findIndex((v) => v.version_id === v1);
      const idx2 = versions.findIndex((v) => v.version_id === v2);
      const [older, newer] = idx1 > idx2 ? [text1, text2] : [text2, text1];
      setDiffResult(diffLines(older, newer));
    } catch (e) {
      setError(String(e));
    } finally {
      setDiffLoading(false);
    }
  }

  async function handleRestoreVersion(versionId: string) {
    if (!versionFile) return;
    setRestoringVersion(true);
    try {
      await restoreFileVersion(clientId, versionFile, versionId);
      handleCloseVersions();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setRestoringVersion(false);
    }
  }

  async function handleRestoreDeletedFile(filename: string, versionId: string) {
    setRestoringDeletedFile(filename);
    try {
      await restoreDeletedFile(clientId, filename, versionId);
      setDeletedFiles((prev) => prev.filter((f) => f.filename !== filename));
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setRestoringDeletedFile(null);
    }
  }

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-2xl mx-auto p-8">
        {/* Error */}
        {error && (
          <div className="bg-red-50 border border-red-200 rounded-lg p-4 mb-6">
            <p className="text-red-800 text-sm">{error}</p>
          </div>
        )}

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
              <button
                onClick={handleToggleMore}
                className={`p-1.5 rounded border transition-colors ${
                  moreMode
                    ? "border-blue-300 bg-blue-50 text-blue-600"
                    : "border-gray-300 text-gray-400 hover:bg-gray-100"
                }`}
                title={moreMode ? "Hide version history" : "Show version history"}
              >
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
                </svg>
              </button>
              <button
                onClick={() => setShowCreateText(true)}
                className="px-3 py-1 text-xs font-medium text-white bg-green-600 rounded hover:bg-green-700 transition-colors"
              >
                Create Text File
              </button>
            </div>
          </div>

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
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                  </svg>
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
                              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z" />
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                              </svg>
                            )}
                          </button>
                          <button
                            onClick={() => setDeleteConfirm(file.filename)}
                            title="Delete chat history"
                            className="p-1.5 text-gray-400 hover:text-red-600 hover:bg-red-50 rounded transition-colors"
                          >
                            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                            </svg>
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
                    <p className="text-sm font-medium text-gray-900 truncate">
                      {file.filename}
                    </p>
                    <p className="text-xs text-gray-400">
                      {formatFileSize(file.size)}
                    </p>
                  </div>
                  <div className="flex gap-1">
                    {moreMode && (
                      <button
                        onClick={() => handleOpenVersions(file.filename)}
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
                          d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                        />
                      </svg>
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

          {/* Drop hint */}
          {!loading && (
            <div
              className={`px-4 py-6 text-center ${
                files.length === 0 && uploading.length === 0 ? "py-12" : ""
              }`}
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
            </div>
          )}
        </div>
      </div>

      {/* Preview modal */}
      {previewText !== null && previewFilename && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-2xl w-full mx-4 p-6 max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-semibold text-gray-900">
                {previewFilename}
              </h3>
              <button
                onClick={() => {
                  setPreviewText(null);
                  setPreviewFilename(null);
                }}
                className="text-gray-400 hover:text-gray-600 transition-colors"
              >
                <svg
                  className="w-5 h-5"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M6 18L18 6M6 6l12 12"
                  />
                </svg>
              </button>
            </div>
            <div className="flex-1 overflow-y-auto border border-gray-200 rounded-lg p-4">
              <pre className="text-sm text-gray-700 whitespace-pre-wrap font-mono">
                {previewText}
              </pre>
            </div>
            <div className="flex justify-end mt-4">
              <button
                onClick={() => {
                  setPreviewText(null);
                  setPreviewFilename(null);
                }}
                className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Edit text file modal */}
      {editText !== null && editFilename && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-2xl w-full mx-4 p-6 max-h-[80vh] flex flex-col">
            <h3 className="text-lg font-semibold text-gray-900 mb-4">
              {editFilename}
            </h3>
            <textarea
              value={editText}
              onChange={(e) => setEditText(e.target.value)}
              disabled={saving}
              className="flex-1 min-h-[300px] w-full px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono resize-none focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:bg-gray-50"
            />
            <div className="flex justify-end gap-3 mt-4">
              <button
                onClick={() => {
                  setEditText(null);
                  setEditFilename(null);
                }}
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
          </div>
        </div>
      )}

      {/* Delete confirmation */}
      {deleteConfirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-sm w-full mx-4 p-6">
            <h3 className="text-lg font-semibold text-gray-900 mb-2">
              Delete file?
            </h3>
            <p className="text-sm text-gray-600 mb-6">
              Delete <span className="font-medium">{deleteConfirm}</span> and
              its extracted text? This cannot be undone.
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
          </div>
        </div>
      )}

      {/* Create text file modal */}
      {showCreateText && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-2xl w-full mx-4 p-6 max-h-[80vh] flex flex-col">
            <h3 className="text-lg font-semibold text-gray-900 mb-4">
              Create Text File
            </h3>
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
                onClick={() => {
                  setShowCreateText(false);
                  setCreateFilename("");
                  setCreateContent("");
                }}
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
          </div>
        </div>
      )}

      {/* Deleted files (More mode) */}
      {moreMode && !loading && (
        <div className="max-w-2xl mx-auto px-8 pb-8">
          <h3 className="text-sm font-semibold text-gray-500 mb-3">Deleted Files</h3>
          {deletedFilesLoading ? (
            <div className="bg-gray-50 border border-gray-200 rounded-lg p-4 text-center">
              <div className="flex items-center justify-center gap-2 text-gray-500 text-sm">
                <Spinner />
                <span>Loading deleted files...</span>
              </div>
            </div>
          ) : deletedFiles.length === 0 ? (
            <div className="bg-gray-50 border border-gray-200 rounded-lg p-4 text-center">
              <p className="text-gray-400 text-sm">No deleted files found.</p>
            </div>
          ) : (
            <div className="bg-white border border-gray-200 rounded-lg overflow-hidden divide-y divide-gray-100">
              {deletedFiles.map((df) => (
                <div key={df.filename} className="px-4 py-3 flex items-center gap-3 opacity-60">
                  <FileIcon filename={df.filename} />
                  <div className="flex-1 min-w-0">
                    <p className="text-sm text-gray-500 line-through truncate">{df.filename}</p>
                    <p className="text-xs text-gray-400">
                      {df.deleted_at ? `Deleted ${formatDate(df.deleted_at)}` : "Deleted"}
                    </p>
                  </div>
                  <button
                    onClick={() => handleRestoreDeletedFile(df.filename, df.version_id)}
                    disabled={restoringDeletedFile === df.filename}
                    className="px-3 py-1 text-xs text-blue-600 border border-blue-300 rounded hover:bg-blue-50 transition-colors disabled:opacity-50"
                  >
                    {restoringDeletedFile === df.filename ? "Restoring..." : "Restore"}
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Version history modal */}
      {versionFile && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-2xl w-full mx-4 p-6 max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-semibold text-gray-900">
                Version History: {versionFile}
              </h3>
              <button
                onClick={handleCloseVersions}
                className="text-gray-400 hover:text-gray-600 transition-colors"
              >
                <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>

            {versionsLoading ? (
              <div className="flex-1 flex items-center justify-center py-8">
                <div className="flex items-center gap-2 text-gray-500 text-sm">
                  <Spinner />
                  <span>Loading versions...</span>
                </div>
              </div>
            ) : versions.length === 0 ? (
              <div className="flex-1 flex items-center justify-center py-8">
                <p className="text-gray-400 text-sm">No version history found.</p>
              </div>
            ) : (
              <div className="flex-1 overflow-y-auto">
                {/* Compare button */}
                <div className="flex items-center justify-between mb-3">
                  <p className="text-xs text-gray-500">
                    {selectedVersions.size === 2
                      ? "2 versions selected"
                      : `Select 2 versions to compare (${selectedVersions.size}/2)`}
                  </p>
                  <button
                    onClick={handleCompare}
                    disabled={selectedVersions.size !== 2 || diffLoading}
                    className="px-3 py-1 text-xs font-medium text-white bg-blue-600 rounded hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    {diffLoading ? "Comparing..." : "Compare"}
                  </button>
                </div>

                {/* Version list */}
                <div className="border border-gray-200 rounded-lg divide-y divide-gray-100">
                  {versions.map((v) => (
                    <div key={v.version_id}>
                      <div className="px-4 py-3 flex items-center gap-3">
                        <input
                          type="checkbox"
                          checked={selectedVersions.has(v.version_id)}
                          onChange={() => handleToggleVersionSelect(v.version_id)}
                          className="rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                        />
                        <div className="flex-1 min-w-0">
                          <p className="text-sm text-gray-900">
                            {v.last_modified ? formatDate(v.last_modified) : "Unknown date"}
                            {v.is_latest && (
                              <span className="ml-2 px-1.5 py-0.5 text-xs bg-green-100 text-green-700 rounded">
                                Current
                              </span>
                            )}
                          </p>
                          <p className="text-xs text-gray-400">
                            {formatFileSize(v.size)} &middot; {v.version_id.slice(0, 12)}...
                          </p>
                        </div>
                        <div className="flex gap-1">
                          <button
                            onClick={() => handleViewVersion(v.version_id)}
                            className={`px-2 py-1 text-xs rounded transition-colors ${
                              versionPreview?.versionId === v.version_id
                                ? "bg-blue-100 text-blue-700"
                                : "text-blue-600 hover:bg-blue-50"
                            }`}
                          >
                            {versionPreviewLoading && versionPreview?.versionId !== v.version_id
                              ? "..."
                              : versionPreview?.versionId === v.version_id
                                ? "Hide"
                                : "View"}
                          </button>
                          {!v.is_latest && (
                            <button
                              onClick={() => handleRestoreVersion(v.version_id)}
                              disabled={restoringVersion}
                              className="px-2 py-1 text-xs text-amber-600 hover:bg-amber-50 rounded transition-colors disabled:opacity-50"
                            >
                              {restoringVersion ? "..." : "Restore"}
                            </button>
                          )}
                        </div>
                      </div>
                      {/* Inline version preview */}
                      {versionPreview?.versionId === v.version_id && (
                        <div className="px-4 pb-3">
                          <pre className="text-xs text-gray-700 whitespace-pre-wrap font-mono bg-gray-50 border border-gray-200 rounded p-3 max-h-[200px] overflow-y-auto">
                            {versionPreview.text}
                          </pre>
                        </div>
                      )}
                    </div>
                  ))}
                </div>

                {/* Diff panel */}
                {diffResult && (
                  <div className="mt-4">
                    <h4 className="text-sm font-semibold text-gray-700 mb-2">Diff</h4>
                    <div className="border border-gray-200 rounded-lg overflow-auto max-h-[7.5rem]">
                      <pre className="text-xs font-mono p-3 whitespace-pre w-max min-w-full">
                        {diffResult.map((line, i) => (
                          <div
                            key={i}
                            className={
                              line.type === "add"
                                ? "bg-green-50 text-green-800"
                                : line.type === "remove"
                                  ? "bg-red-50 text-red-800"
                                  : "text-gray-600"
                            }
                          >
                            <span className="select-none inline-block w-4 text-gray-400 mr-2">
                              {line.type === "add" ? "+" : line.type === "remove" ? "-" : " "}
                            </span>
                            {line.spans
                              ? line.spans.map((span, si) => (
                                  <span
                                    key={si}
                                    className={
                                      span.highlight
                                        ? line.type === "add"
                                          ? "bg-green-200 rounded-sm"
                                          : "bg-red-200 rounded-sm"
                                        : ""
                                    }
                                  >
                                    {span.text}
                                  </span>
                                ))
                              : line.line}
                          </div>
                        ))}
                      </pre>
                    </div>
                  </div>
                )}
              </div>
            )}

            <div className="flex justify-end mt-4">
              <button
                onClick={handleCloseVersions}
                className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

const AUDIO_EXTENSIONS = new Set([
  "mp3", "mp4", "m4a", "wav", "flac", "ogg", "amr", "webm",
]);

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

function formatDate(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return iso;
  }
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function Spinner() {
  return (
    <svg
      className="animate-spin h-3.5 w-3.5"
      viewBox="0 0 24 24"
      fill="none"
    >
      <circle
        className="opacity-25"
        cx="12"
        cy="12"
        r="10"
        stroke="currentColor"
        strokeWidth="4"
      />
      <path
        className="opacity-75"
        fill="currentColor"
        d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
      />
    </svg>
  );
}
