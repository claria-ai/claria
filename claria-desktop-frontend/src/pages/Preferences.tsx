import { useState, useEffect, useCallback } from "react";
import {
  getSystemPrompt,
  saveSystemPrompt,
  deleteSystemPrompt,
  setPreferredModel,
  listSystemPromptVersions,
  getSystemPromptVersion,
  restoreSystemPromptVersion,
  type ChatModel,
  type FileVersion,
} from "../lib/tauri";
import type { Page } from "../App";

export default function Preferences({
  navigate,
  chatModels,
  chatModelsLoading,
  chatModelsError,
  preferredModelId,
  onPreferredModelChanged,
}: {
  navigate: (page: Page) => void;
  chatModels: ChatModel[];
  chatModelsLoading: boolean;
  chatModelsError: string | null;
  preferredModelId: string | null;
  onPreferredModelChanged: (id: string | null) => void;
}) {
  // System prompt state
  const [promptContent, setPromptContent] = useState("");
  const [promptLoading, setPromptLoading] = useState(true);
  const [promptSaving, setPromptSaving] = useState(false);
  const [promptError, setPromptError] = useState<string | null>(null);
  const [promptDirty, setPromptDirty] = useState(false);

  // System prompt version history state
  const [showVersions, setShowVersions] = useState(false);
  const [versions, setVersions] = useState<FileVersion[]>([]);
  const [versionsLoading, setVersionsLoading] = useState(false);
  const [versionPreview, setVersionPreview] = useState<{
    versionId: string;
    text: string;
  } | null>(null);
  const [versionPreviewLoading, setVersionPreviewLoading] = useState(false);
  const [restoringVersion, setRestoringVersion] = useState(false);

  // Model preference state
  const [modelSaving, setModelSaving] = useState(false);

  const loadPrompt = useCallback(async () => {
    setPromptLoading(true);
    setPromptError(null);
    try {
      const content = await getSystemPrompt();
      setPromptContent(content);
      setPromptDirty(false);
    } catch (e) {
      setPromptError(String(e));
    } finally {
      setPromptLoading(false);
    }
  }, []);

  useEffect(() => {
    loadPrompt();
  }, [loadPrompt]);

  async function handleSavePrompt() {
    setPromptSaving(true);
    setPromptError(null);
    try {
      await saveSystemPrompt(promptContent);
      setPromptDirty(false);
    } catch (e) {
      setPromptError(String(e));
    } finally {
      setPromptSaving(false);
    }
  }

  async function handleResetPrompt() {
    setPromptSaving(true);
    setPromptError(null);
    try {
      await deleteSystemPrompt();
      const content = await getSystemPrompt();
      setPromptContent(content);
      setPromptDirty(false);
    } catch (e) {
      setPromptError(String(e));
    } finally {
      setPromptSaving(false);
    }
  }

  async function handleOpenVersions() {
    setShowVersions(true);
    setVersionsLoading(true);
    setVersionPreview(null);
    try {
      setVersions(await listSystemPromptVersions());
    } catch (e) {
      setPromptError(String(e));
    } finally {
      setVersionsLoading(false);
    }
  }

  function handleCloseVersions() {
    setShowVersions(false);
    setVersions([]);
    setVersionPreview(null);
  }

  async function handleViewVersion(versionId: string) {
    if (versionPreview?.versionId === versionId) {
      setVersionPreview(null);
      return;
    }
    setVersionPreviewLoading(true);
    try {
      const text = await getSystemPromptVersion(versionId);
      setVersionPreview({ versionId, text });
    } catch (e) {
      setPromptError(String(e));
    } finally {
      setVersionPreviewLoading(false);
    }
  }

  async function handleRestoreVersion(versionId: string) {
    setRestoringVersion(true);
    try {
      await restoreSystemPromptVersion(versionId);
      handleCloseVersions();
      await loadPrompt();
    } catch (e) {
      setPromptError(String(e));
    } finally {
      setRestoringVersion(false);
    }
  }

  async function handleModelChange(modelId: string) {
    const value = modelId || null;
    setModelSaving(true);
    try {
      await setPreferredModel(value);
      onPreferredModelChanged(value);
    } catch (e) {
      setPromptError(String(e));
    } finally {
      setModelSaving(false);
    }
  }

  return (
    <div className="max-w-2xl mx-auto p-8">
      {/* Header */}
      <div className="flex items-center gap-3 mb-6">
        <button
          onClick={() => navigate("start")}
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
        <h2 className="text-2xl font-bold">Preferences</h2>
      </div>

      <div className="space-y-4">
        {/* System Prompt section */}
        <details className="border border-gray-200 rounded-lg group" open>
          <summary className="flex items-center justify-between p-4 cursor-pointer list-none [&::-webkit-details-marker]:hidden">
            <span className="font-medium text-gray-900">System Prompt</span>
            <span className="shrink-0 text-gray-400 text-xs transition-transform group-open:rotate-90">
              &#9656;
            </span>
          </summary>
          <div className="border-t border-gray-100 p-4">
            {promptLoading ? (
              <div className="flex items-center justify-center py-8">
                <div className="flex items-center gap-2 text-gray-500 text-sm">
                  <Spinner />
                  <span>Loading prompt...</span>
                </div>
              </div>
            ) : (
              <>
                <textarea
                  value={promptContent}
                  onChange={(e) => {
                    setPromptContent(e.target.value);
                    setPromptDirty(true);
                  }}
                  disabled={promptSaving}
                  className="w-full min-h-[200px] px-3 py-2 text-sm font-mono border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent resize-y disabled:bg-gray-50"
                />

                {promptError && (
                  <div className="bg-red-50 border border-red-200 rounded-lg p-3 mt-3">
                    <p className="text-red-800 text-sm">{promptError}</p>
                  </div>
                )}

                <div className="flex justify-between mt-3">
                  <div className="flex gap-2">
                    <button
                      onClick={handleResetPrompt}
                      disabled={promptLoading || promptSaving}
                      className="px-3 py-1.5 text-sm text-amber-600 border border-amber-300 rounded-lg hover:bg-amber-50 transition-colors disabled:opacity-50"
                    >
                      {promptSaving ? "Resetting..." : "Reset to Default"}
                    </button>
                    <button
                      onClick={handleOpenVersions}
                      disabled={promptSaving}
                      className="px-3 py-1.5 text-sm text-gray-600 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors disabled:opacity-50"
                    >
                      Version History
                    </button>
                  </div>
                  <button
                    onClick={handleSavePrompt}
                    disabled={promptLoading || promptSaving || !promptDirty}
                    className="px-4 py-1.5 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
                  >
                    {promptSaving ? "Saving..." : "Save"}
                  </button>
                </div>
              </>
            )}
          </div>
        </details>

        {/* Preferred Model section */}
        <details className="border border-gray-200 rounded-lg group">
          <summary className="flex items-center justify-between p-4 cursor-pointer list-none [&::-webkit-details-marker]:hidden">
            <div className="flex items-center gap-2">
              <span className="font-medium text-gray-900">Preferred Model</span>
              {preferredModelId && chatModels.length > 0 && (
                <span className="text-xs text-gray-400">
                  {chatModels.find((m) => m.model_id === preferredModelId)
                    ?.name ?? preferredModelId}
                </span>
              )}
            </div>
            <span className="shrink-0 text-gray-400 text-xs transition-transform group-open:rotate-90">
              &#9656;
            </span>
          </summary>
          <div className="border-t border-gray-100 p-4">
            {chatModelsLoading ? (
              <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
                <Spinner />
                <span>Loading models...</span>
              </div>
            ) : chatModelsError ? (
              <div className="bg-red-50 border border-red-200 rounded-lg p-3">
                <p className="text-red-800 text-sm">{chatModelsError}</p>
              </div>
            ) : (
              <>
                <select
                  value={preferredModelId ?? ""}
                  onChange={(e) => handleModelChange(e.target.value)}
                  disabled={modelSaving}
                  className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:bg-gray-50"
                >
                  <option value="">Use first available model</option>
                  {chatModels.map((m) => (
                    <option key={m.model_id} value={m.model_id}>
                      {m.name}
                    </option>
                  ))}
                </select>
                <p className="text-xs text-gray-400 mt-2">
                  Applies to new chat sessions. Existing chats keep the model
                  they were started with.
                </p>
              </>
            )}
          </div>
        </details>
      </div>

      {/* Version history modal */}
      {showVersions && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-2xl w-full mx-4 p-6 max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-semibold text-gray-900">
                System Prompt Versions
              </h3>
              <button
                onClick={handleCloseVersions}
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

            {versionsLoading ? (
              <div className="flex-1 flex items-center justify-center py-8">
                <div className="flex items-center gap-2 text-gray-500 text-sm">
                  <Spinner />
                  <span>Loading versions...</span>
                </div>
              </div>
            ) : versions.length === 0 ? (
              <div className="flex-1 flex items-center justify-center py-8">
                <p className="text-gray-400 text-sm">
                  No version history found.
                </p>
              </div>
            ) : (
              <div className="flex-1 overflow-y-auto">
                <div className="border border-gray-200 rounded-lg divide-y divide-gray-100">
                  {versions.map((v) => (
                    <div key={v.version_id}>
                      <div className="px-4 py-3 flex items-center gap-3">
                        <div className="flex-1 min-w-0">
                          <p className="text-sm text-gray-900">
                            {v.last_modified
                              ? formatDate(v.last_modified)
                              : "Unknown date"}
                            {v.is_latest && (
                              <span className="ml-2 px-1.5 py-0.5 text-xs bg-green-100 text-green-700 rounded">
                                Current
                              </span>
                            )}
                          </p>
                          <p className="text-xs text-gray-400">
                            {formatFileSize(v.size)} &middot;{" "}
                            {v.version_id.slice(0, 12)}...
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
                            {versionPreviewLoading &&
                            versionPreview?.versionId !== v.version_id
                              ? "..."
                              : versionPreview?.versionId === v.version_id
                                ? "Hide"
                                : "View"}
                          </button>
                          {!v.is_latest && (
                            <button
                              onClick={() =>
                                handleRestoreVersion(v.version_id)
                              }
                              disabled={restoringVersion}
                              className="px-2 py-1 text-xs text-amber-600 hover:bg-amber-50 rounded transition-colors disabled:opacity-50"
                            >
                              {restoringVersion ? "..." : "Restore"}
                            </button>
                          )}
                        </div>
                      </div>
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
              </div>
            )}
          </div>
        </div>
      )}
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
    <svg className="animate-spin h-3.5 w-3.5" viewBox="0 0 24 24" fill="none">
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
