import { useState, useEffect, useCallback } from "react";
import {
  getPrompt,
  savePrompt,
  deletePrompt,
  setPreferredModel,
  listPromptVersions,
  getPromptVersion,
  restorePromptVersion,
  getWhisperModels,
  downloadWhisperModel,
  deleteWhisperModel,
  deleteWhisperModelDir,
  setActiveWhisperModel,
  loadConfig,
  setHourlyCostData,
  getCostAndUsage,
  type ChatModel,
  type FileVersion,
  type WhisperModelInfo,
  type WhisperModelTier,
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
  // Model preference state
  const [modelSaving, setModelSaving] = useState(false);
  const [modelError, setModelError] = useState<string | null>(null);

  async function handleModelChange(modelId: string) {
    const value = modelId || null;
    setModelSaving(true);
    setModelError(null);
    try {
      await setPreferredModel(value);
      onPreferredModelChanged(value);
    } catch (e) {
      setModelError(String(e));
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
        <PromptEditor
          promptName="system-prompt"
          label="System Prompt"
          description="Instructions given to the AI assistant at the start of every chat session."
          defaultOpen
        />

        {/* PDF Extraction Prompt section */}
        <PromptEditor
          promptName="pdf-extraction"
          label="PDF Extraction Prompt"
          description="Instructions used when extracting text from uploaded PDF and DOCX files."
        />

        {/* Memo Transcription section */}
        <MemoTranscriptionSection />

        {/* Cost Explorer section */}
        <CostExplorerSection />

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
                {modelError && (
                  <div className="bg-red-50 border border-red-200 rounded-lg p-3 mt-3">
                    <p className="text-red-800 text-sm">{modelError}</p>
                  </div>
                )}
                <p className="text-xs text-gray-400 mt-2">
                  Applies to new chat sessions. Existing chats keep the model
                  they were started with.
                </p>
              </>
            )}
          </div>
        </details>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Reusable prompt editor accordion
// ---------------------------------------------------------------------------

function PromptEditor({
  promptName,
  label,
  description,
  defaultOpen,
}: {
  promptName: string;
  label: string;
  description: string;
  defaultOpen?: boolean;
}) {
  const [content, setContent] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);

  // Version history state
  const [showVersions, setShowVersions] = useState(false);
  const [versions, setVersions] = useState<FileVersion[]>([]);
  const [versionsLoading, setVersionsLoading] = useState(false);
  const [versionPreview, setVersionPreview] = useState<{
    versionId: string;
    text: string;
  } | null>(null);
  const [versionPreviewLoading, setVersionPreviewLoading] = useState(false);
  const [restoringVersion, setRestoringVersion] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const text = await getPrompt(promptName);
      setContent(text);
      setDirty(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [promptName]);

  useEffect(() => {
    load();
  }, [load]);

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      await savePrompt(promptName, content);
      setDirty(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleReset() {
    setSaving(true);
    setError(null);
    try {
      await deletePrompt(promptName);
      const text = await getPrompt(promptName);
      setContent(text);
      setDirty(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleOpenVersions() {
    setShowVersions(true);
    setVersionsLoading(true);
    setVersionPreview(null);
    try {
      setVersions(await listPromptVersions(promptName));
    } catch (e) {
      setError(String(e));
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
      const text = await getPromptVersion(promptName, versionId);
      setVersionPreview({ versionId, text });
    } catch (e) {
      setError(String(e));
    } finally {
      setVersionPreviewLoading(false);
    }
  }

  async function handleRestoreVersion(versionId: string) {
    setRestoringVersion(true);
    try {
      await restorePromptVersion(promptName, versionId);
      handleCloseVersions();
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setRestoringVersion(false);
    }
  }

  return (
    <>
      <details
        className="border border-gray-200 rounded-lg group"
        open={defaultOpen}
      >
        <summary className="flex items-center justify-between p-4 cursor-pointer list-none [&::-webkit-details-marker]:hidden">
          <span className="font-medium text-gray-900">{label}</span>
          <span className="shrink-0 text-gray-400 text-xs transition-transform group-open:rotate-90">
            &#9656;
          </span>
        </summary>
        <div className="border-t border-gray-100 p-4">
          {description && (
            <p className="text-xs text-gray-400 mb-3">{description}</p>
          )}
          {loading ? (
            <div className="flex items-center justify-center py-8">
              <div className="flex items-center gap-2 text-gray-500 text-sm">
                <Spinner />
                <span>Loading prompt...</span>
              </div>
            </div>
          ) : (
            <>
              <textarea
                value={content}
                onChange={(e) => {
                  setContent(e.target.value);
                  setDirty(true);
                }}
                disabled={saving}
                className="w-full min-h-[200px] px-3 py-2 text-sm font-mono border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent resize-y disabled:bg-gray-50"
              />

              {error && (
                <div className="bg-red-50 border border-red-200 rounded-lg p-3 mt-3">
                  <p className="text-red-800 text-sm">{error}</p>
                </div>
              )}

              <div className="flex justify-between mt-3">
                <div className="flex gap-2">
                  <button
                    onClick={handleReset}
                    disabled={loading || saving}
                    className="px-3 py-1.5 text-sm text-amber-600 border border-amber-300 rounded-lg hover:bg-amber-50 transition-colors disabled:opacity-50"
                  >
                    {saving ? "Resetting..." : "Reset to Default"}
                  </button>
                  <button
                    onClick={handleOpenVersions}
                    disabled={saving}
                    className="px-3 py-1.5 text-sm text-gray-600 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors disabled:opacity-50"
                  >
                    Version History
                  </button>
                </div>
                <button
                  onClick={handleSave}
                  disabled={loading || saving || !dirty}
                  className="px-4 py-1.5 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
                >
                  {saving ? "Saving..." : "Save"}
                </button>
              </div>
            </>
          )}
        </div>
      </details>

      {/* Version history modal */}
      {showVersions && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-2xl w-full mx-4 p-6 max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-semibold text-gray-900">
                {label} Versions
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
    </>
  );
}

// ---------------------------------------------------------------------------
// Memo Transcription model management
// ---------------------------------------------------------------------------

function MemoTranscriptionSection() {
  const [models, setModels] = useState<WhisperModelInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyTier, setBusyTier] = useState<WhisperModelTier | null>(null);
  const [busyDir, setBusyDir] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setModels(await getWhisperModels());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function handleDownload(tier: WhisperModelTier) {
    setBusyTier(tier);
    setError(null);
    try {
      setModels(await downloadWhisperModel(tier));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyTier(null);
    }
  }

  async function handleDelete(tier: WhisperModelTier) {
    setBusyTier(tier);
    setError(null);
    try {
      setModels(await deleteWhisperModel(tier));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyTier(null);
    }
  }

  async function handleDeleteDir(dirName: string) {
    setBusyDir(dirName);
    setError(null);
    try {
      setModels(await deleteWhisperModelDir(dirName));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyDir(null);
    }
  }

  async function handleActivate(tier: WhisperModelTier) {
    setError(null);
    try {
      setModels(await setActiveWhisperModel(tier));
    } catch (e) {
      setError(String(e));
    }
  }

  const isBusy = busyTier !== null || busyDir !== null;
  const knownModels = models.filter((m) => m.tier !== null);
  const orphanModels = models.filter((m) => m.tier === null);
  const hasActive = models.some((m) => m.active);

  return (
    <details className="border border-gray-200 rounded-lg group">
      <summary className="flex items-center justify-between p-4 cursor-pointer list-none [&::-webkit-details-marker]:hidden">
        <div className="flex items-center gap-2">
          <span className="font-medium text-gray-900">Memo Transcription</span>
          {hasActive && (
            <span className="text-xs text-green-600">Ready</span>
          )}
        </div>
        <span className="shrink-0 text-gray-400 text-xs transition-transform group-open:rotate-90">
          &#9656;
        </span>
      </summary>
      <div className="border-t border-gray-100 p-4">
        <p className="text-xs text-gray-400 mb-3">
          Record audio memos and transcribe them to text notes using a local AI
          model. No audio data leaves your computer. Download one or more models
          below and activate the one you want to use.
        </p>

        {loading ? (
          <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
            <Spinner />
            <span>Checking model status...</span>
          </div>
        ) : (
          <div className="space-y-3">
            {knownModels.map((m) => (
              <div
                key={m.dir_name}
                className={`border rounded-lg p-3 ${
                  m.active
                    ? "border-green-300 bg-green-50/50"
                    : "border-gray-200"
                }`}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="font-medium text-sm text-gray-900">
                        {m.label}
                      </span>
                      {m.active && (
                        <span className="px-1.5 py-0.5 text-xs bg-green-100 text-green-700 rounded">
                          Active
                        </span>
                      )}
                    </div>
                    <p className="text-xs text-gray-500 mt-0.5">
                      {m.description}
                    </p>
                    {m.downloaded && (
                      <div className="text-xs text-gray-400 mt-1 space-y-0.5">
                        {m.model_size_bytes != null && (
                          <p>Size on disk: {formatFileSize(m.model_size_bytes)}</p>
                        )}
                        {m.model_path && (
                          <p className="break-all">Location: {m.model_path}</p>
                        )}
                      </div>
                    )}
                  </div>
                  <div className="flex items-center gap-2 shrink-0">
                    {m.downloaded ? (
                      <>
                        {!m.active && m.tier && (
                          <button
                            onClick={() => handleActivate(m.tier!)}
                            disabled={isBusy}
                            className="px-2.5 py-1 text-xs text-blue-600 border border-blue-300 rounded-lg hover:bg-blue-50 transition-colors disabled:opacity-50"
                          >
                            Activate
                          </button>
                        )}
                        {m.tier && (
                          <button
                            onClick={() => handleDelete(m.tier!)}
                            disabled={isBusy}
                            className="px-2.5 py-1 text-xs text-red-600 border border-red-300 rounded-lg hover:bg-red-50 transition-colors disabled:opacity-50"
                          >
                            {busyTier === m.tier ? "Removing..." : "Remove"}
                          </button>
                        )}
                      </>
                    ) : m.tier ? (
                      <button
                        onClick={() => handleDownload(m.tier!)}
                        disabled={isBusy}
                        className="px-2.5 py-1 text-xs text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 flex items-center gap-1.5"
                      >
                        {busyTier === m.tier ? (
                          <>
                            <Spinner />
                            <span>Downloading...</span>
                          </>
                        ) : (
                          `Download (${m.download_size})`
                        )}
                      </button>
                    ) : null}
                  </div>
                </div>
              </div>
            ))}

            {orphanModels.length > 0 && (
              <>
                <p className="text-xs text-gray-500 mt-2 pt-2 border-t border-gray-100">
                  Other models on disk
                </p>
                {orphanModels.map((m) => (
                  <div
                    key={m.dir_name}
                    className="border border-amber-200 bg-amber-50/50 rounded-lg p-3"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="flex-1 min-w-0">
                        <span className="font-medium text-sm text-gray-900">
                          {m.label}
                        </span>
                        <p className="text-xs text-gray-500 mt-0.5">
                          {m.description}
                        </p>
                        <div className="text-xs text-gray-400 mt-1 space-y-0.5">
                          {m.model_size_bytes != null && (
                            <p>Size on disk: {formatFileSize(m.model_size_bytes)}</p>
                          )}
                          {m.model_path && (
                            <p className="break-all">Location: {m.model_path}</p>
                          )}
                        </div>
                      </div>
                      <button
                        onClick={() => handleDeleteDir(m.dir_name)}
                        disabled={isBusy}
                        className="px-2.5 py-1 text-xs text-red-600 border border-red-300 rounded-lg hover:bg-red-50 transition-colors disabled:opacity-50 shrink-0"
                      >
                        {busyDir === m.dir_name ? "Removing..." : "Remove"}
                      </button>
                    </div>
                  </div>
                ))}
              </>
            )}
          </div>
        )}

        {error && (
          <div className="bg-red-50 border border-red-200 rounded-lg p-3 mt-3">
            <p className="text-red-800 text-sm">{error}</p>
          </div>
        )}
      </div>
    </details>
  );
}

// ---------------------------------------------------------------------------
// Cost Explorer settings
// ---------------------------------------------------------------------------

function CostExplorerSection() {
  const [hourlyEnabled, setHourlyEnabled] = useState<boolean | null>(null);
  const [loading, setLoading] = useState(true);
  const [verifying, setVerifying] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    loadConfig()
      .then((info) => setHourlyEnabled(info.hourly_cost_data))
      .catch(() => setHourlyEnabled(false))
      .finally(() => setLoading(false));
  }, []);

  async function handleToggle() {
    if (hourlyEnabled) {
      // Turning off — no verification needed
      setError(null);
      try {
        await setHourlyCostData(false);
        setHourlyEnabled(false);
      } catch (e) {
        setError(String(e));
      }
      return;
    }

    // Turning on — verify with a test hourly request
    setVerifying(true);
    setError(null);
    try {
      const yesterday = new Date();
      yesterday.setDate(yesterday.getDate() - 1);
      const today = new Date();
      const fmt = (d: Date) => {
        const y = d.getFullYear();
        const m = String(d.getMonth() + 1).padStart(2, "0");
        const day = String(d.getDate()).padStart(2, "0");
        return `${y}-${m}-${day}`;
      };
      await getCostAndUsage(fmt(yesterday), fmt(today), "hourly", false);
      await setHourlyCostData(true);
      setHourlyEnabled(true);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("AccessDenied") || msg.includes("access denied")) {
        setError(
          "Hourly data is not enabled for this account. In the AWS Console, go to " +
            "Billing → Cost Explorer → Settings and enable \"Hourly and Resource Level Data\"."
        );
      } else if (msg.includes("DataUnavailable") || msg.includes("not enabled")) {
        setError(
          "Hourly cost data is not available yet. Enable it in the AWS Console under " +
            "Billing → Cost Explorer → Settings, then wait up to 24 hours for data to appear."
        );
      } else {
        setError(msg);
      }
    } finally {
      setVerifying(false);
    }
  }

  return (
    <details className="border border-gray-200 rounded-lg group">
      <summary className="flex items-center justify-between p-4 cursor-pointer list-none [&::-webkit-details-marker]:hidden">
        <div className="flex items-center gap-2">
          <span className="font-medium text-gray-900">Cost Explorer</span>
          {hourlyEnabled && (
            <span className="text-xs text-gray-400">Hourly enabled</span>
          )}
        </div>
        <span className="shrink-0 text-gray-400 text-xs transition-transform group-open:rotate-90">
          &#9656;
        </span>
      </summary>
      <div className="border-t border-gray-100 p-4">
        <p className="text-xs text-gray-400 mb-3">
          AWS Cost Explorer charges $0.01 per API request. Hourly-resolution data
          requires separate enablement in the AWS Console and incurs additional
          storage costs on your AWS bill.
        </p>

        {loading ? (
          <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
            <Spinner />
            <span>Loading...</span>
          </div>
        ) : (
          <label className="flex items-start gap-3">
            <input
              type="checkbox"
              checked={hourlyEnabled ?? false}
              onChange={handleToggle}
              disabled={verifying}
              className="mt-0.5 rounded border-gray-300"
            />
            <div className="flex-1">
              <span className="text-sm text-gray-900">
                Hourly data resolution
                {verifying && (
                  <span className="ml-2 text-xs text-gray-400 inline-flex items-center gap-1">
                    <Spinner /> Verifying...
                  </span>
                )}
              </span>
              <p className="text-xs text-gray-400 mt-0.5">
                Shows hourly cost breakdowns for the last 14 days. Must be enabled in
                AWS Console under Billing &rarr; Cost Explorer &rarr; Settings first.
              </p>
            </div>
          </label>
        )}

        {error && (
          <div className="bg-red-50 border border-red-200 rounded-lg p-3 mt-3">
            <p className="text-red-800 text-sm">{error}</p>
          </div>
        )}
      </div>
    </details>
  );
}

// ---------------------------------------------------------------------------
// Shared utilities
// ---------------------------------------------------------------------------

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
