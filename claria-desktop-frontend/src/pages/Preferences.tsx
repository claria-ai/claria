import { useState, useEffect, useCallback, useRef } from "react";
import {
  getPrompt,
  getWriterTrustRules,
  savePrompt,
  deletePrompt,
  setPreferredModel,
  getLocalTranscriptionStatus,
  downloadLocalModel,
  deleteLocalModel,
  deleteLegacyTranscriptionModels,
  saveLocalTranscriptionSettings,
  loadConfig,
  setHourlyCostData,
  getCostAndUsage,
  savePreferencesPatch,
  fetchCloudPreferences,
  uploadWriterTemplate,
  renameWriterTemplate,
  deleteWriterTemplate,
  saveWriterLibraryPrompt,
  deleteWriterLibraryPrompt,
  type ConfigInfo,
  type EffortPreference,
  type ModelTuningPreferences,
  type LocalBackend,
  type LocalKvPrecision,
  type LocalModelId,
  type LocalModelInfo,
  type LocalTranscriptionSettings,
  type LocalTranscriptionStatus,
  type ModelDownloadProgress,
  type ReportAuthoringPreferences,
  type TranscriptionLanguage,
  type TranscriptionPreferences,
  type WriterPrompt,
  type WriterTemplateView,
} from "../lib/tauri";
import { logFrontendEvent } from "../lib/logBridge";
import { useChatModels } from "../lib/chatModels";
import { costErrorMessage } from "../lib/costErrors";
import { useAsyncLoad } from "../lib/useAsyncLoad";
import { useWriterPrompts } from "../lib/useWriterPrompts";
import { useWriterTemplates } from "../lib/useWriterTemplates";
import { formatDateTime, formatFileSize } from "../lib/format";
import { promptVersions } from "../lib/versions";
import EditableName from "../components/EditableName";
import PreferencesSection from "../components/PreferencesSection";
import { BackButton, TrashIcon } from "../components/icons";
import Spinner from "../components/Spinner";
import { ErrorBanner, LoadingCard } from "../components/StateCards";
import VersionHistoryModal from "../components/VersionHistoryModal";
import type { Page, PreferencesWriterSection } from "../App";

export default function Preferences({
  navigate,
  focusSection = null,
  backPage = "start",
}: {
  navigate: (page: Page) => void;
  focusSection?: PreferencesWriterSection | null;
  backPage?: Page;
}) {
  const {
    models: chatModels,
    loading: chatModelsLoading,
    error: chatModelsError,
    preferredModelId,
    setPreferredModelId,
  } = useChatModels();

  // Fixed writer trust rules, displayed read-only under the writer prompt
  // editors so nothing about how the writer runs is hidden.
  const { data: trustRules } = useAsyncLoad(() => getWriterTrustRules(), []);

  // Model preference state
  const [modelSaving, setModelSaving] = useState(false);
  const [modelError, setModelError] = useState<string | null>(null);

  async function handleModelChange(modelId: string) {
    const value = modelId || null;
    setModelSaving(true);
    setModelError(null);
    try {
      await setPreferredModel(value);
      setPreferredModelId(value);
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
        <BackButton onClick={() => navigate(backPage)} />
        <h2 className="text-2xl font-bold">Preferences</h2>
      </div>

      {/* Cross-machine sync notice — these settings live in S3 and follow the
         clinician across computers. Other running copies of Claria won't see
         changes until restart. */}
      <div className="mb-4 bg-blue-50 border border-blue-200 rounded-lg p-3">
        <p className="text-sm text-blue-900">
          Shared defaults are stored in your S3 bucket so they sync across
          computers. Local memo models, decoder controls, and hardware choices
          are machine-local.
        </p>
      </div>

      <div className="space-y-4">
        {/* Transcription preferences section */}
        <TranscriptionSection />

        {/* Managed, redacted templates used by the document writer */}
        <WriterTemplatesSection defaultOpen={focusSection === "writer-templates"} />

        {/* Saved steering prompts that prefill writer instructions */}
        <WriterPromptsSection defaultOpen={focusSection === "writer-prompts"} />

        {/* Agentic document-writer guardrails */}
        <ReportAuthoringSection />

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
          description="Instructions used when converting uploaded PDF and DOCX files to structured Markdown."
        />

        {/* Writer prompt sections — the editable bodies of the document
            writer's system prompts. Claria always appends fixed trust rules,
            shown read-only so nothing about how the writer runs is hidden. */}
        <PromptEditor
          promptName="report-system"
          label="Writer Prompt"
          description="Instructions given to the document writer for targeted edits and proposals."
          fixedRules={trustRules?.targeted}
        />
        <PromptEditor
          promptName="report-full-draft"
          label="Whole-Report Prompt"
          description="Instructions used when filling the complete report in one action."
          fixedRules={trustRules?.full_draft}
        />

        {/* Machine-local transcribe.cpp models and inference controls */}
        <LocalTranscriptionSection />

        {/* Cost Explorer section */}
        <CostExplorerSection />

        {/* Preferred Model section */}
        <PreferencesSection
          title="Preferred Model"
          summary={
            preferredModelId && chatModels.length > 0 ? (
              <span className="text-xs text-gray-400">
                {chatModels.find((m) => m.model_id === preferredModelId)
                  ?.name ?? preferredModelId}
              </span>
            ) : undefined
          }
        >
            {chatModelsLoading ? (
              <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
                <Spinner />
                <span>Loading models...</span>
              </div>
            ) : chatModelsError ? (
              <ErrorBanner message={chatModelsError} className="" />
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
                  <ErrorBanner message={modelError} className="mt-3" />
                )}
                <p className="text-xs text-gray-400 mt-2">
                  Applies to new chat sessions. Existing chats keep the model
                  they were started with.
                </p>
              </>
            )}
        </PreferencesSection>

        {/* Opt-in model tuning knobs (adaptive reasoning, effort, temperature) */}
        <ModelTuningSection />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Model tuning — opt-in adaptive reasoning, effort, and temperature
// ---------------------------------------------------------------------------

const EFFORT_OPTIONS: { value: "" | EffortPreference; label: string }[] = [
  { value: "", label: "Model default (high)" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "max", label: "Max" },
];

function ModelTuningSection() {
  const [snapshot, setSnapshot] = useState<ConfigInfo | null>(null);
  const [draft, setDraft] = useState<ModelTuningPreferences | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const info = await fetchCloudPreferences();
      setSnapshot(info);
      setDraft(info.model_tuning);
    } catch (e) {
      try {
        const info = await loadConfig();
        setSnapshot(info);
        setDraft(info.model_tuning);
      } catch (fallbackError) {
        setLoadError(String(fallbackError ?? e));
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const dirty =
    snapshot != null &&
    draft != null &&
    JSON.stringify(snapshot.model_tuning) !== JSON.stringify(draft);
  const temperatureInvalid =
    draft?.temperature != null &&
    (draft.temperature < 0 || draft.temperature > 1);

  async function save() {
    if (!snapshot || !draft || temperatureInvalid) return;
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      // Patch-save: only the model tuning section travels.
      const updated = await savePreferencesPatch({ model_tuning: draft });
      setSnapshot(updated);
      setDraft(updated.model_tuning);
      setSaved(true);
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <PreferencesSection
      title="Model Tuning"
      summary={
        draft && (draft.reasoning_enabled || draft.effort || draft.temperature != null) ? (
          <span className="text-xs text-gray-400">
            {[
              draft.reasoning_enabled ? "Adaptive reasoning on" : null,
              draft.effort ? `effort: ${draft.effort}` : null,
              draft.temperature != null ? `temp: ${draft.temperature}` : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </span>
        ) : undefined
      }
      contentClassName="border-t border-gray-100 p-4 space-y-4"
    >
      {loading ? (
        <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
          <Spinner />
          <span>Loading model tuning...</span>
        </div>
      ) : !draft ? (
        <ErrorBanner
          message={loadError ?? "Could not load model tuning preferences."}
          className=""
        />
      ) : (
        <>
          <p className="text-xs text-gray-500">
            Optional controls applied to chat and writer requests. Each
            setting is sent only to models that support it — unsupported
            knobs are skipped automatically, so nothing here can break a
            model that does not accept them.
          </p>

          <label className="flex items-start gap-2">
            <input
              type="checkbox"
              checked={draft.reasoning_enabled}
              onChange={(e) => {
                setSaved(false);
                setDraft({ ...draft, reasoning_enabled: e.target.checked });
              }}
              className="mt-0.5"
            />
            <span>
              <span className="text-sm text-gray-700 block">
                Adaptive reasoning
              </span>
              <span className="text-xs text-gray-500 block">
                Lets supported models (Claude 4.6 and newer) think before
                answering, which can improve report quality. Reasoning tokens
                bill as output and count against the response budget, so
                turns cost more and take longer.
              </span>
            </span>
          </label>

          <div>
            <label className="text-sm text-gray-700 block mb-1">Effort</label>
            <select
              value={draft.effort ?? ""}
              onChange={(e) => {
                setSaved(false);
                setDraft({
                  ...draft,
                  effort: (e.target.value || null) as EffortPreference | null,
                });
              }}
              className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            >
              {EFFORT_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
            <p className="text-xs text-gray-500 mt-1">
              How much reasoning supported models put into each response.
              Sent to Opus 4.5 and every model from Claude 4.6 on.
            </p>
          </div>

          <div>
            <label className="text-sm text-gray-700 block mb-1">
              Temperature
            </label>
            <input
              type="number"
              min={0}
              max={1}
              step={0.1}
              placeholder="Model default"
              value={draft.temperature ?? ""}
              onChange={(e) => {
                setSaved(false);
                setDraft({
                  ...draft,
                  temperature:
                    e.target.value === "" ? null : Number(e.target.value),
                });
              }}
              className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            />
            <p className="text-xs text-gray-500 mt-1">
              0.0 to 1.0; leave blank for the model default. Only sent to
              model generations that accept it (through Claude 4.6) — newer
              models always use their own default.
            </p>
          </div>

          {temperatureInvalid && (
            <ErrorBanner
              message="Temperature must be between 0.0 and 1.0."
              className=""
            />
          )}
          {saveError && <ErrorBanner message={saveError} className="" />}

          <div className="flex items-center justify-end gap-3">
            {saved && !dirty && (
              <span className="text-xs text-green-600">Saved</span>
            )}
            <button
              onClick={save}
              disabled={!dirty || saving || temperatureInvalid}
              className="px-4 py-1.5 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
            >
              {saving ? "Saving..." : "Save"}
            </button>
          </div>
        </>
      )}
    </PreferencesSection>
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
  fixedRules,
}: {
  promptName: string;
  label: string;
  description: string;
  defaultOpen?: boolean;
  /** Read-only rules Claria always appends after the editable body. */
  fixedRules?: string;
}) {
  const [content, setContent] = useState("");
  const [saving, setSaving] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);

  const [showVersions, setShowVersions] = useState(false);

  const {
    data: loadedContent,
    loading,
    error: loadError,
    reload,
  } = useAsyncLoad(() => getPrompt(promptName), [promptName]);
  const error = loadError ?? actionError;

  // Adopt each freshly loaded prompt as the editable copy.
  useEffect(() => {
    if (loadedContent != null) {
      setContent(loadedContent);
      setDirty(false);
    }
  }, [loadedContent]);

  async function handleSave() {
    setSaving(true);
    setActionError(null);
    try {
      await savePrompt(promptName, content);
      setDirty(false);
    } catch (e) {
      setActionError(String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleReset() {
    setSaving(true);
    setActionError(null);
    try {
      await deletePrompt(promptName);
      const text = await getPrompt(promptName);
      setContent(text);
      setDirty(false);
    } catch (e) {
      setActionError(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <>
      <PreferencesSection title={label} defaultOpen={defaultOpen}>
          {description && (
            <p className="text-xs text-gray-400 mb-3">{description}</p>
          )}
          {loading ? (
            <LoadingCard>Loading prompt...</LoadingCard>
          ) : (
            <>
              <textarea
                value={content}
                onChange={(e) => {
                  setContent(e.target.value);
                  setDirty(true);
                }}
                disabled={saving}
                className="w-full min-h-[216px] px-3 py-2 text-sm font-mono border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent resize-y disabled:bg-gray-50"
              />

              {fixedRules && (
                <div className="mt-3">
                  <p className="text-xs text-gray-400 mb-1">
                    Claria always appends these trust rules after your prompt.
                    They are not editable and cannot be removed:
                  </p>
                  {/* 184px = 8px top padding + 11 exact 16px text-xs line
                      boxes, so the scroll fold never slices a line. */}
                  <pre className="w-full max-h-[184px] overflow-auto px-3 py-2 text-xs font-mono text-gray-500 bg-gray-50 border border-gray-200 rounded-lg whitespace-pre-wrap">
                    {fixedRules}
                  </pre>
                </div>
              )}

              {error && (
                <ErrorBanner message={error} className="mt-3" />
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
                    onClick={() => setShowVersions(true)}
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
      </PreferencesSection>

      {showVersions && (
        <VersionHistoryModal
          title={`${label} Versions`}
          source={promptVersions(promptName)}
          onClose={() => setShowVersions(false)}
          onRestored={reload}
          onError={setActionError}
        />
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// transcribe.cpp model management and machine-local inference settings
// ---------------------------------------------------------------------------

function LocalTranscriptionSection() {
  const [status, setStatus] = useState<LocalTranscriptionStatus | null>(null);
  const [draft, setDraft] = useState<LocalTranscriptionSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [busyModel, setBusyModel] = useState<LocalModelId | null>(null);
  const [removingLegacy, setRemovingLegacy] = useState(false);
  const [progress, setProgress] = useState<ModelDownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const applyStatus = useCallback((next: LocalTranscriptionStatus) => {
    setStatus(next);
    setDraft(next.settings);
  }, []);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      applyStatus(await getLocalTranscriptionStatus());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [applyStatus]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function persist(next: LocalTranscriptionSettings) {
    setSaving(true);
    setError(null);
    try {
      applyStatus(await saveLocalTranscriptionSettings(next));
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleDownload(modelId: LocalModelId) {
    setBusyModel(modelId);
    setProgress(null);
    setError(null);
    try {
      applyStatus(
        await downloadLocalModel(modelId, (next) => setProgress(next)),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyModel(null);
      setProgress(null);
    }
  }

  async function handleDelete(modelId: LocalModelId) {
    setBusyModel(modelId);
    setError(null);
    try {
      applyStatus(await deleteLocalModel(modelId));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyModel(null);
    }
  }

  async function handleLegacyDelete() {
    setRemovingLegacy(true);
    setError(null);
    try {
      applyStatus(await deleteLegacyTranscriptionModels());
    } catch (e) {
      setError(String(e));
    } finally {
      setRemovingLegacy(false);
    }
  }

  function activate(model: LocalModelInfo) {
    if (!draft) return;
    void persist({ ...draft, speech_model: model.id });
  }

  const dirty =
    status != null &&
    draft != null &&
    JSON.stringify(status.settings) !== JSON.stringify(draft);
  const ready =
    status?.models.some((model) => model.active && model.downloaded) ?? false;
  const selectedBackendKind =
    draft?.backend === "cpu_accel" ? "accel" : draft?.backend;
  const selectedDevice =
    status && draft
      ? draft.gpu_device > 0
        ? status.devices.find((device) => device.index === draft.gpu_device)
        : status.devices.find((device) =>
            selectedBackendKind === "auto"
              ? !["cpu", "accel"].includes(device.kind)
              : device.kind === selectedBackendKind,
          )
      : undefined;
  const maxDeviceIndex = Math.max(
    0,
    ...(status?.devices.map((device) => device.index ?? 0) ?? []),
  );

  return (
    <PreferencesSection
      title="On-device Memo Transcription"
      summary={ready ? <span className="text-xs text-green-600">Ready</span> : undefined}
      defaultOpen
      contentClassName="border-t border-gray-100 p-4 space-y-5"
    >
        <p className="text-xs text-gray-500">
          Record Memo uses transcribe.cpp and local GGUF models, so microphone
          audio stays on this computer. Imported audio recordings continue to
          use Amazon Transcribe. These model and hardware settings are
          machine-local.
        </p>

        {loading ? (
          <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
            <Spinner /> <span>Checking local runtime and models...</span>
          </div>
        ) : status && draft ? (
          <>
            <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 text-xs text-gray-600 space-y-1">
              <p>
                transcribe.cpp {status.runtime_version} · {status.accelerated ? "GPU accelerated" : "CPU"}
              </p>
              {selectedDevice && (
                <p>
                  {selectedDevice.description || selectedDevice.name}
                  {selectedDevice.memory_total > 0
                    ? ` · ${formatFileSize(selectedDevice.memory_total)} memory`
                    : ""}
                </p>
              )}
            </div>

            <ModelGroup
              title="Memo speech model"
              description="Used only for on-device Record Memo transcription."
              models={status.models}
              busyModel={busyModel}
              progress={progress}
              disabled={saving || removingLegacy || dirty}
              onActivate={activate}
              onDownload={handleDownload}
              onDelete={handleDelete}
            />

            <div className="pt-1 border-t border-gray-100 space-y-3">
              <h4 className="text-sm font-medium text-gray-700">Compute</h4>
              <div className="grid grid-cols-2 gap-3">
                <label className="text-xs text-gray-600">
                  Backend
                  <select
                    value={draft.backend}
                    onChange={(event) =>
                      setDraft({ ...draft, backend: event.target.value as LocalBackend })
                    }
                    className="mt-1 w-full px-2.5 py-2 text-sm border border-gray-300 rounded-lg bg-white"
                  >
                    {status.backends.map((backend) => (
                      <option
                        key={backend.backend}
                        value={backend.backend}
                        disabled={!backend.available}
                      >
                        {backend.label}{backend.available ? "" : " (unavailable)"}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="text-xs text-gray-600">
                  Compute device index (0 = automatic)
                  <input
                    type="number"
                    min={0}
                    max={maxDeviceIndex}
                    value={draft.gpu_device}
                    onChange={(event) =>
                      setDraft({ ...draft, gpu_device: Number(event.target.value) })
                    }
                    className="mt-1 w-full px-2.5 py-2 text-sm border border-gray-300 rounded-lg"
                  />
                </label>
                <label className="text-xs text-gray-600">
                  CPU threads (0 = automatic)
                  <input
                    type="number"
                    min={0}
                    max={256}
                    value={draft.cpu_threads}
                    onChange={(event) =>
                      setDraft({ ...draft, cpu_threads: Number(event.target.value) })
                    }
                    className="mt-1 w-full px-2.5 py-2 text-sm border border-gray-300 rounded-lg"
                  />
                </label>
                <label className="text-xs text-gray-600">
                  K/V cache precision
                  <select
                    value={draft.kv_precision}
                    onChange={(event) =>
                      setDraft({
                        ...draft,
                        kv_precision: event.target.value as LocalKvPrecision,
                      })
                    }
                    className="mt-1 w-full px-2.5 py-2 text-sm border border-gray-300 rounded-lg bg-white"
                  >
                    <option value="auto">Automatic</option>
                    <option value="f16">F16 (lower memory)</option>
                    <option value="f32">F32 (higher precision)</option>
                  </select>
                </label>
                {status.devices.length > 0 && (
                  <p className="col-span-2 text-xs text-gray-400">
                    Runtime devices: {status.devices.map((device) =>
                      `${device.index ?? "?"}: ${device.description || device.name}`,
                    ).join(" · ")}
                  </p>
                )}
              </div>
            </div>

            <details className="border border-gray-200 rounded-lg">
              <summary className="p-3 cursor-pointer text-sm font-medium text-gray-700">
                Advanced Whisper decoding
              </summary>
              <div className="border-t border-gray-100 p-3 space-y-3">
                <label className="block text-xs text-gray-600">
                  Initial prompt / vocabulary hint
                  <textarea
                    value={draft.initial_prompt}
                    onChange={(event) =>
                      setDraft({ ...draft, initial_prompt: event.target.value })
                    }
                    placeholder="Optional clinical terms, names, or context"
                    className="mt-1 w-full min-h-20 px-2.5 py-2 text-sm border border-gray-300 rounded-lg"
                  />
                </label>
                <label className="flex items-start gap-2 text-sm text-gray-700">
                  <input
                    type="checkbox"
                    checked={draft.condition_on_previous_text}
                    onChange={(event) =>
                      setDraft({
                        ...draft,
                        condition_on_previous_text: event.target.checked,
                      })
                    }
                    className="mt-0.5"
                  />
                  Carry accepted text into each following 30-second window
                </label>
                <div className="grid grid-cols-2 gap-3">
                  <NumberSetting
                    label="Previous-context tokens"
                    value={draft.max_previous_context_tokens}
                    min={0}
                    max={448}
                    step={1}
                    onChange={(value) =>
                      setDraft({ ...draft, max_previous_context_tokens: value })
                    }
                  />
                  <NumberSetting
                    label="Temperature"
                    value={draft.temperature}
                    min={0}
                    max={1}
                    step={0.1}
                    onChange={(value) => setDraft({ ...draft, temperature: value })}
                  />
                  <NumberSetting
                    label="Temperature increment"
                    value={draft.temperature_increment}
                    min={0}
                    max={1}
                    step={0.1}
                    onChange={(value) =>
                      setDraft({ ...draft, temperature_increment: value })
                    }
                  />
                  <NumberSetting
                    label="Compression-ratio threshold"
                    value={draft.compression_ratio_threshold}
                    min={0.1}
                    max={100}
                    step={0.1}
                    onChange={(value) =>
                      setDraft({ ...draft, compression_ratio_threshold: value })
                    }
                  />
                  <NumberSetting
                    label="Log-probability threshold"
                    value={draft.log_probability_threshold}
                    min={-100}
                    max={0}
                    step={0.1}
                    onChange={(value) =>
                      setDraft({ ...draft, log_probability_threshold: value })
                    }
                  />
                  <NumberSetting
                    label="No-speech threshold"
                    value={draft.no_speech_threshold}
                    min={0}
                    max={1}
                    step={0.05}
                    onChange={(value) =>
                      setDraft({ ...draft, no_speech_threshold: value })
                    }
                  />
                  <NumberSetting
                    label="Sampling seed (0 = random)"
                    value={draft.seed}
                    min={0}
                    max={4294967295}
                    step={1}
                    onChange={(value) => setDraft({ ...draft, seed: value })}
                  />
                </div>
              </div>
            </details>

            <div className="flex justify-end">
              <button
                onClick={() => void persist(draft)}
                disabled={!dirty || saving || busyModel !== null}
                className="px-3 py-1.5 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50"
              >
                {saving ? "Saving..." : "Save local engine settings"}
              </button>
            </div>

            {status.legacy_model_bytes > 0 && (
              <div className="border border-amber-200 bg-amber-50 rounded-lg p-3 flex items-start justify-between gap-3">
                <div>
                  <p className="text-sm text-amber-900">Legacy Candle model files</p>
                  <p className="text-xs text-amber-700 mt-0.5">
                    {formatFileSize(status.legacy_model_bytes)} from the previous
                    safetensors engine is no longer used.
                  </p>
                </div>
                <button
                  onClick={() => void handleLegacyDelete()}
                  disabled={removingLegacy || busyModel !== null || dirty}
                  className="px-2.5 py-1 text-xs text-red-700 border border-red-300 rounded-lg hover:bg-red-50 disabled:opacity-50"
                >
                  {removingLegacy ? "Removing..." : "Remove legacy files"}
                </button>
              </div>
            )}
          </>
        ) : null}

        {error && (
          <ErrorBanner
            message={error}
            onRetry={() => void refresh()}
            className=""
          />
        )}
    </PreferencesSection>
  );
}

function ModelGroup({
  title,
  description,
  models,
  busyModel,
  progress,
  disabled,
  onActivate,
  onDownload,
  onDelete,
}: {
  title: string;
  description: string;
  models: LocalModelInfo[];
  busyModel: LocalModelId | null;
  progress: ModelDownloadProgress | null;
  disabled: boolean;
  onActivate: (model: LocalModelInfo) => void;
  onDownload: (modelId: LocalModelId) => void;
  onDelete: (modelId: LocalModelId) => void;
}) {
  return (
    <div>
      <h4 className="text-sm font-medium text-gray-700">{title}</h4>
      <p className="text-xs text-gray-500 mt-0.5 mb-2">{description}</p>
      <div className="space-y-2">
        {models.map((model) => {
          const modelProgress = progress?.model_id === model.id ? progress : null;
          const percent = modelProgress
            ? Math.min(
                100,
                Math.round(
                  (modelProgress.downloaded_bytes / modelProgress.total_bytes) * 100,
                ),
              )
            : 0;
          return (
            <div
              key={model.id}
              className={`border rounded-lg p-3 ${
                model.active ? "border-green-300 bg-green-50/40" : "border-gray-200"
              }`}
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium text-gray-900">{model.label}</span>
                    {model.active && (
                      <span className="px-1.5 py-0.5 text-xs bg-green-100 text-green-700 rounded">
                        Active
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-gray-500 mt-0.5">{model.description}</p>
                  <p className="text-xs text-gray-400 mt-1">
                    {model.quantization} · {formatFileSize(model.download_size_bytes)} · {model.languages.join(", ")}
                  </p>
                  {model.model_path && (
                    <p className="text-xs text-gray-400 mt-0.5 break-all">{model.model_path}</p>
                  )}
                </div>
                <div className="flex gap-2 shrink-0">
                  {model.downloaded ? (
                    <>
                      {!model.active && (
                        <button
                          onClick={() => onActivate(model)}
                          disabled={disabled || busyModel !== null}
                          className="px-2.5 py-1 text-xs text-blue-600 border border-blue-300 rounded-lg disabled:opacity-50"
                        >
                          Activate
                        </button>
                      )}
                      <button
                        onClick={() => onDelete(model.id)}
                        disabled={disabled || busyModel !== null}
                        className="px-2.5 py-1 text-xs text-red-600 border border-red-300 rounded-lg disabled:opacity-50"
                      >
                        {busyModel === model.id ? "Removing..." : "Remove"}
                      </button>
                    </>
                  ) : (
                    <button
                      onClick={() => onDownload(model.id)}
                      disabled={disabled || busyModel !== null}
                      className="px-2.5 py-1 text-xs text-white bg-blue-600 rounded-lg disabled:opacity-50"
                    >
                      {busyModel === model.id ? `Downloading ${percent}%` : "Download"}
                    </button>
                  )}
                </div>
              </div>
              {modelProgress && (
                <div className="mt-2 h-1.5 bg-gray-200 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-blue-600 transition-[width]"
                    style={{ width: `${percent}%` }}
                  />
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function NumberSetting({
  label,
  value,
  min,
  max,
  step,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="text-xs text-gray-600">
      {label}
      <input
        type="number"
        value={value}
        min={min}
        max={max}
        step={step}
        onChange={(event) => onChange(Number(event.target.value))}
        className="mt-1 w-full px-2.5 py-2 text-sm border border-gray-300 rounded-lg"
      />
    </label>
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
      setError(
        costErrorMessage(e, {
          accessDenied:
            "Hourly data is not enabled for this account. In the AWS Console, go to " +
            'Billing → Cost Explorer → Settings and enable "Hourly and Resource Level Data".',
          dataUnavailable:
            "Hourly cost data is not available yet. Enable it in the AWS Console under " +
            "Billing → Cost Explorer → Settings, then wait up to 24 hours for data to appear.",
        })
      );
    } finally {
      setVerifying(false);
    }
  }

  return (
    <PreferencesSection
      title="Cost Explorer"
      summary={
        hourlyEnabled ? (
          <span className="text-xs text-gray-400">Hourly enabled</span>
        ) : undefined
      }
    >
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
          <ErrorBanner message={error} className="mt-3" />
        )}
    </PreferencesSection>
  );
}

// ---------------------------------------------------------------------------
// Writer templates: managed redacted DOCX presets stored in S3
// ---------------------------------------------------------------------------

function WriterTemplatesSection({ defaultOpen }: { defaultOpen: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  const {
    templates,
    setTemplates,
    loading,
    error: loadError,
    reload,
  } = useWriterTemplates();
  const [busy, setBusy] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const error = loadError ?? actionError;

  useEffect(() => {
    if (defaultOpen) setOpen(true);
  }, [defaultOpen]);

  async function upload() {
    setBusy("upload");
    setActionError(null);
    try {
      const uploaded = await uploadWriterTemplate();
      if (uploaded) {
        setTemplates((current) => [uploaded, ...current]);
      }
    } catch (reason) {
      setActionError(String(reason));
    } finally {
      setBusy(null);
    }
  }

  async function rename(templateId: string, name: string) {
    const updated = await renameWriterTemplate(templateId, name);
    setTemplates((current) =>
      current.map((template) => (template.id === templateId ? updated : template))
    );
  }

  async function remove(template: WriterTemplateView) {
    if (!window.confirm(`Delete ${template.name}? Existing reports are not affected.`)) {
      return;
    }
    setBusy(template.id);
    setActionError(null);
    try {
      await deleteWriterTemplate(template.id);
      setTemplates((current) =>
        current.filter((candidate) => candidate.id !== template.id)
      );
    } catch (reason) {
      setActionError(String(reason));
    } finally {
      setBusy(null);
    }
  }

  return (
    <PreferencesSection
      title="Writer Templates"
      summary={
        <span className="text-xs text-gray-400">{templates.length} saved</span>
      }
      open={open}
      onToggle={setOpen}
      testId="writer-template-manager"
      contentClassName="border-t border-gray-100 p-4 space-y-4"
    >
        <div className="flex items-start justify-between gap-4">
          <div>
            <p className="text-sm text-gray-700">
              Keep a small shelf of reusable Word templates in Claria&apos;s managed
              S3 storage. Writing sessions can preview and apply these presets.
            </p>
            <p className="text-xs text-amber-700 mt-1">
              Upload only redacted templates. Remove names, dates, diagnoses,
              scores, and other client-specific facts before saving a preset.
            </p>
          </div>
          <button
            type="button"
            onClick={() => void upload()}
            disabled={busy !== null}
            className="shrink-0 px-3 py-1.5 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50"
          >
            {busy === "upload" ? "Uploading…" : "Upload .docx"}
          </button>
        </div>

        {loading ? (
          <div className="flex items-center gap-2 text-sm text-gray-500">
            <Spinner /> Loading writer templates…
          </div>
        ) : templates.length === 0 ? (
          <p className="rounded-lg border border-dashed border-gray-300 p-4 text-center text-sm text-gray-500">
            No writer templates yet.
          </p>
        ) : (
          <div className="divide-y divide-gray-100 rounded-lg border border-gray-200">
            {templates.map((template) => (
              <div key={template.id} className="flex items-center gap-3 p-3">
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded bg-blue-50 text-xs font-bold text-blue-700">
                  W
                </div>
                <div className="min-w-0 flex-1">
                  <EditableName
                    value={template.name}
                    label="writer template"
                    onSave={(name) => rename(template.id, name)}
                    disabled={busy !== null}
                    className="w-full"
                  />
                  <p className="mt-0.5 text-xs text-gray-400">
                    {formatFileSize(template.size)} · uploaded{" "}
                    {formatDateTime(template.uploaded_at)} · used {template.use_count}{" "}
                    time{template.use_count === 1 ? "" : "s"}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => void remove(template)}
                  disabled={busy !== null}
                  aria-label={`Delete ${template.name}`}
                  title="Delete writer template"
                  className="rounded p-1.5 text-gray-400 hover:bg-red-50 hover:text-red-600 disabled:opacity-40"
                >
                  <TrashIcon />
                </button>
              </div>
            ))}
          </div>
        )}

        {error && (
          <ErrorBanner
            message={error}
            onRetry={() => {
              setActionError(null);
              void reload();
            }}
            className=""
          />
        )}
    </PreferencesSection>
  );
}

function WriterPromptsSection({ defaultOpen }: { defaultOpen: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  const { prompts, setPrompts, loading, error: loadError, reload } = useWriterPrompts();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  /** null = closed, "new" = creating, otherwise the id being edited. */
  const [editing, setEditing] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [draftBody, setDraftBody] = useState("");
  const error = loadError ?? actionError;

  function openEditor(prompt: WriterPrompt | null) {
    setEditing(prompt?.id ?? "new");
    setDraftName(prompt?.name ?? "");
    setDraftBody(prompt?.body ?? "");
    setActionError(null);
  }

  async function saveDraft() {
    setBusy(true);
    setActionError(null);
    try {
      const saved = await saveWriterLibraryPrompt(
        editing === "new" ? null : editing,
        draftName,
        draftBody
      );
      setPrompts((current) => {
        const rest = current.filter((prompt) => prompt.id !== saved.id);
        return [...rest, saved].sort((left, right) =>
          left.name.toLowerCase().localeCompare(right.name.toLowerCase())
        );
      });
      setEditing(null);
    } catch (reason) {
      setActionError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function remove(prompt: WriterPrompt) {
    if (!window.confirm(`Delete the saved prompt "${prompt.name}"?`)) {
      return;
    }
    setBusy(true);
    setActionError(null);
    try {
      await deleteWriterLibraryPrompt(prompt.id);
      setPrompts((current) => current.filter((candidate) => candidate.id !== prompt.id));
      if (editing === prompt.id) setEditing(null);
    } catch (reason) {
      setActionError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <PreferencesSection
      title="Prompt Library"
      summary={<span className="text-xs text-gray-400">{prompts.length} saved</span>}
      open={open}
      onToggle={setOpen}
      testId="writer-prompt-manager"
      contentClassName="border-t border-gray-100 p-4 space-y-4"
    >
      <div className="flex items-start justify-between gap-4">
        <div>
          <p className="text-sm text-gray-700">
            Save the writer instructions you reuse — one for each phase of a
            report. Picking one in a Writing session fills the instruction
            box, where you can still edit it before sending.
          </p>
          <p className="text-xs text-amber-700 mt-1">
            Saved prompts are shared across all clients. Write placeholders
            like $DIAGNOSIS instead of client names or other identifying
            details.
          </p>
        </div>
        <button
          type="button"
          onClick={() => openEditor(null)}
          disabled={busy || editing !== null}
          className="shrink-0 px-3 py-1.5 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50"
        >
          New prompt
        </button>
      </div>

      {editing !== null && (
        <div
          className="rounded-lg border border-blue-200 bg-blue-50/40 p-3 space-y-2"
          data-testid="writer-prompt-editor"
        >
          <input
            type="text"
            value={draftName}
            onChange={(event) => setDraftName(event.target.value)}
            placeholder="Prompt name (e.g. Phase 1 — history sections)"
            aria-label="Saved prompt name"
            className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm"
          />
          <textarea
            value={draftBody}
            onChange={(event) => setDraftBody(event.target.value)}
            placeholder="The instruction to prefill, e.g. Fill in Reason for Referral, Background, and Medical/Social History from the records; skip everything else."
            aria-label="Saved prompt text"
            rows={5}
            className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm font-mono"
          />
          <div className="flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setEditing(null)}
              disabled={busy}
              className="px-3 py-1.5 text-sm text-gray-600 hover:text-gray-900 disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => void saveDraft()}
              disabled={busy || draftName.trim() === "" || draftBody.trim() === ""}
              className="px-3 py-1.5 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50"
            >
              {busy ? "Saving…" : "Save prompt"}
            </button>
          </div>
        </div>
      )}

      {loading ? (
        <div className="flex items-center gap-2 text-sm text-gray-500">
          <Spinner /> Loading saved prompts…
        </div>
      ) : prompts.length === 0 && editing === null ? (
        <p className="rounded-lg border border-dashed border-gray-300 p-4 text-center text-sm text-gray-500">
          No saved prompts yet.
        </p>
      ) : (
        prompts.length > 0 && (
          <div className="divide-y divide-gray-100 rounded-lg border border-gray-200">
            {prompts.map((prompt) => (
              <div key={prompt.id} className="flex items-center gap-3 p-3">
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium text-gray-900">
                    {prompt.name}
                  </p>
                  <p className="mt-0.5 truncate text-xs text-gray-400">
                    {prompt.body}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => openEditor(prompt)}
                  disabled={busy || editing !== null}
                  className="rounded px-2 py-1 text-xs font-medium text-gray-600 hover:bg-gray-100 disabled:opacity-40"
                >
                  Edit
                </button>
                <button
                  type="button"
                  onClick={() => void remove(prompt)}
                  disabled={busy}
                  aria-label={`Delete ${prompt.name}`}
                  title="Delete saved prompt"
                  className="rounded p-1.5 text-gray-400 hover:bg-red-50 hover:text-red-600 disabled:opacity-40"
                >
                  <TrashIcon />
                </button>
              </div>
            ))}
          </div>
        )
      )}

      {error && (
        <ErrorBanner
          message={error}
          onRetry={() => {
            setActionError(null);
            void reload();
          }}
          className=""
        />
      )}
    </PreferencesSection>
  );
}

// ---------------------------------------------------------------------------
// Document writer: configurable agentic-loop guardrails
// ---------------------------------------------------------------------------

type WriterLimits = Required<ReportAuthoringPreferences>;
type WriterLimitField = keyof WriterLimits;

const WRITER_LIMIT_DEFAULTS: WriterLimits = {
  max_tool_rounds: 40,
  max_converse_calls: 50,
  max_tool_uses_per_response: 80,
  max_retained_turns: 200,
};

// The `label` values below are quoted verbatim in the writer's
// guardrail-exhausted error, which tells the clinician which field to raise.
// Renaming one here means renaming it in `claria-report-authoring`'s
// `TOOL_ROUNDS_FIELD_LABEL` / `CONVERSE_CALLS_FIELD_LABEL`.
const WRITER_LIMIT_FIELDS: Array<{
  key: WriterLimitField;
  label: string;
  description: string;
  min: number;
  max: number;
}> = [
  {
    key: "max_tool_rounds",
    label: "Tool-use rounds per request",
    description:
      "How many times the writer may request tools and receive their results before it must finish.",
    min: 1,
    max: 100,
  },
  {
    key: "max_converse_calls",
    label: "Bedrock calls per request",
    description:
      "Total billed model-call ceiling. Whichever call or tool-round guardrail is reached first stops the request.",
    min: 1,
    max: 101,
  },
  {
    key: "max_tool_uses_per_response",
    label: "Tool calls per response",
    description:
      "Maximum list, read, and proposal calls accepted from one model response.",
    min: 1,
    max: 100,
  },
  {
    key: "max_retained_turns",
    label: "Conversation turns retained",
    description:
      "Completed writer turns kept as context. The 512 KiB context-history ceiling may prune older turns sooner.",
    min: 1,
    max: 200,
  },
];

function normalizeWriterPreferences(
  value: ReportAuthoringPreferences | null | undefined
): WriterLimits {
  return {
    max_tool_rounds:
      value?.max_tool_rounds ?? WRITER_LIMIT_DEFAULTS.max_tool_rounds,
    max_converse_calls:
      value?.max_converse_calls ?? WRITER_LIMIT_DEFAULTS.max_converse_calls,
    max_tool_uses_per_response:
      value?.max_tool_uses_per_response ??
      WRITER_LIMIT_DEFAULTS.max_tool_uses_per_response,
    max_retained_turns:
      value?.max_retained_turns ?? WRITER_LIMIT_DEFAULTS.max_retained_turns,
  };
}

function writerLimitsError(value: WriterLimits): string | null {
  for (const field of WRITER_LIMIT_FIELDS) {
    const input = value[field.key];
    if (!Number.isInteger(input) || input < field.min || input > field.max) {
      return `${field.label} must be a whole number from ${field.min} to ${field.max}.`;
    }
  }
  return null;
}

function ReportAuthoringSection() {
  const [snapshot, setSnapshot] = useState<ConfigInfo | null>(null);
  const [draft, setDraft] = useState<WriterLimits | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const info = await fetchCloudPreferences();
      setSnapshot(info);
      setDraft(normalizeWriterPreferences(info.report_authoring));
    } catch (e) {
      try {
        const info = await loadConfig();
        setSnapshot(info);
        setDraft(normalizeWriterPreferences(info.report_authoring));
      } catch (fallbackError) {
        setLoadError(String(fallbackError ?? e));
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const original = snapshot
    ? normalizeWriterPreferences(snapshot.report_authoring)
    : null;
  const dirty =
    original != null &&
    draft != null &&
    JSON.stringify(original) !== JSON.stringify(draft);
  const validationError = draft ? writerLimitsError(draft) : null;
  const effectiveToolRounds = draft
    ? Math.min(
        draft.max_tool_rounds,
        Math.max(0, draft.max_converse_calls - 1)
      )
    : 0;
  const effectiveModelCalls = draft
    ? Math.min(draft.max_converse_calls, draft.max_tool_rounds + 1)
    : 0;
  const theoreticalToolCalls = draft
    ? effectiveToolRounds * draft.max_tool_uses_per_response
    : 0;

  async function save() {
    if (!snapshot || !draft || validationError) return;
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      // Patch-save: only the writer limits travel, so this section cannot
      // roll back a model, cost, or transcription edit.
      const updated = await savePreferencesPatch({ report_authoring: draft });
      setSnapshot(updated);
      setDraft(normalizeWriterPreferences(updated.report_authoring));
      setSaved(true);
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <PreferencesSection
      title="Document Writer Limits"
      summary={
        draft ? (
          <span className="text-xs text-gray-400">
            {draft.max_tool_rounds} rounds · {draft.max_converse_calls} calls
          </span>
        ) : undefined
      }
      contentClassName="border-t border-gray-100 p-4 space-y-4"
    >
        {loading ? (
          <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
            <Spinner />
            <span>Loading document writer limits...</span>
          </div>
        ) : !draft ? (
          <ErrorBanner
            message={loadError ?? "Could not load document writer limits."}
            className=""
          />
        ) : (
          <>
            <div className="bg-amber-50 border border-amber-300 rounded-lg p-3 space-y-1.5">
              <p className="text-sm font-medium text-amber-950">
                Higher limits increase cost and runaway-loop exposure
              </p>
              <p className="text-xs text-amber-900">
                These are spend and runtime guardrails, not targets. With this
                combination, one request may make up to {effectiveModelCalls} billed
                Bedrock calls and theoretically issue{" "}
                {theoreticalToolCalls.toLocaleString()} tool calls. It can run much
                longer, repeatedly read client records, and cost substantially more.
              </p>
              <p className="text-xs text-amber-900">
                Opus is generally reliable, but no model is guaranteed not to
                repeat tools or fail to finish. Writer changes still remain
                proposals until you explicitly accept them.
              </p>
            </div>

            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              {WRITER_LIMIT_FIELDS.map((field) => (
                <label key={field.key} className="block">
                  <span className="text-sm font-medium text-gray-700">
                    {field.label}
                  </span>
                  <input
                    type="number"
                    min={field.min}
                    max={field.max}
                    step={1}
                    value={draft[field.key]}
                    onChange={(event) => {
                      const next = event.currentTarget.valueAsNumber;
                      setDraft({
                        ...draft,
                        [field.key]: Number.isFinite(next) ? next : 0,
                      });
                      setSaved(false);
                    }}
                    className="mt-1 w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  />
                  <span className="block text-xs text-gray-500 mt-1">
                    {field.description}
                  </span>
                </label>
              ))}
            </div>

            {validationError && (
              <ErrorBanner message={validationError} className="" />
            )}
            {saveError && (
              <ErrorBanner
                message={`Could not save document writer limits: ${saveError}`}
                className=""
              />
            )}

            <div className="pt-2 border-t border-gray-100 flex items-center justify-between gap-3">
              <button
                type="button"
                onClick={() => {
                  setDraft(WRITER_LIMIT_DEFAULTS);
                  setSaved(false);
                }}
                disabled={saving}
                className="px-3 py-1.5 text-sm text-gray-600 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors disabled:opacity-50"
              >
                Restore recommended defaults
              </button>
              <div className="flex items-center gap-2">
                {saved && !dirty && (
                  <span className="text-xs text-green-700">Saved</span>
                )}
                <button
                  type="button"
                  onClick={save}
                  disabled={saving || !dirty || validationError != null}
                  className="px-3 py-1.5 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
                >
                  {saving ? "Saving..." : "Save limits"}
                </button>
              </div>
            </div>
          </>
        )}
    </PreferencesSection>
  );
}

// ---------------------------------------------------------------------------
// Transcription section: language / speaker count / engine / translation
// ---------------------------------------------------------------------------

/** How long editing settles before a change is pushed to S3. */
const TRANSCRIPTION_SYNC_DEBOUNCE_MS = 600;

/**
 * Transcription defaults applied to drag-and-drop audio uploads. The wizard
 * uses these as starting values too, but lets the user override per file.
 *
 * Cross-machine sync: on mount we call `fetchCloudPreferences` to pull the
 * latest values from S3 (so the editing machine sees its own recent changes
 * without an app restart). Edits accumulate in a draft and are patch-saved to
 * local config and S3 via `savePreferencesPatch` shortly after the user stops
 * changing things — only this section's fields travel, so sibling sections
 * are never clobbered.
 */
function TranscriptionSection() {
  // The full set of synced fields, fetched on mount.
  const [snapshot, setSnapshot] = useState<ConfigInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Editable copy of the transcription section only.
  const [draft, setDraft] = useState<TranscriptionPreferences | null>(null);
  const dirty =
    snapshot != null &&
    draft != null &&
    JSON.stringify(snapshot.transcription) !== JSON.stringify(draft);

  // Sync status, shown under the controls.
  const [syncing, setSyncing] = useState(false);
  const [syncError, setSyncError] = useState<string | null>(null);
  const [syncedOnce, setSyncedOnce] = useState(false);

  const sync = useCallback(async (next: TranscriptionPreferences) => {
    setSyncing(true);
    setSyncError(null);
    try {
      // Patch-save: only this section's fields travel, so the model
      // dropdown and Cost Explorer sections can never be rolled back by a
      // stale snapshot here.
      const updated = await savePreferencesPatch({ transcription: next });
      // Advancing the snapshot clears `dirty` and stops the debounce.
      setSnapshot(updated);
      setSyncedOnce(true);
    } catch (e) {
      setSyncError(String(e));
    } finally {
      setSyncing(false);
    }
  }, []);

  // Debounced save-on-change. Saving while the screen is still mounted is what
  // makes the edit survive a quit or a window close — an unmount cleanup never
  // runs in either case, so edits used to be lost silently.
  const pendingRef = useRef<TranscriptionPreferences | null>(null);
  useEffect(() => {
    if (!dirty || !snapshot || !draft) {
      pendingRef.current = null;
      return;
    }
    pendingRef.current = draft;
    const id = setTimeout(() => {
      pendingRef.current = null;
      void sync(draft);
    }, TRANSCRIPTION_SYNC_DEBOUNCE_MS);
    return () => clearTimeout(id);
  }, [dirty, snapshot, draft, sync]);

  // Backstop for the one case the debounce cannot cover: navigating away
  // within the debounce window cancels the timer above. Fire-and-forget,
  // because there is no longer any UI to report a failure into — but the
  // window is a few hundred milliseconds, not the whole session.
  useEffect(() => {
    return () => {
      const pending = pendingRef.current;
      if (!pending) return;
      savePreferencesPatch({ transcription: pending }).catch((e) =>
        logFrontendEvent("error", `preferences sync on leave failed: ${e}`)
      );
    };
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const info = await fetchCloudPreferences();
      setSnapshot(info);
      setDraft(info.transcription);
    } catch (e) {
      // Fall back to whatever's in local config — fetchCloudPreferences
      // requires a configured SDK; without one we still want to render.
      try {
        const info = await loadConfig();
        setSnapshot(info);
        setDraft(info.transcription);
      } catch (e2) {
        setError(String(e2 ?? e));
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  function handleLanguageChange(value: TranscriptionLanguage) {
    if (!draft) return;
    setDraft({ ...draft, default_language: value });
  }

  function handleSpeakerCountChange(value: number) {
    if (!draft) return;
    setDraft({ ...draft, default_speaker_count: value });
  }

  function handleMedicalToggle(value: boolean) {
    if (!draft) return;
    setDraft({ ...draft, use_medical_for_english: value });
  }

  function handleTranslateToggle(value: boolean) {
    if (!draft) return;
    setDraft({ ...draft, translate_to_english: value });
  }

  return (
    <PreferencesSection
      title="Imported Audio Transcription"
      summary={
        draft ? (
          <span className="text-xs text-gray-400">
            {labelForLanguage(draft.default_language ?? "english")} ·{" "}
            {draft.default_speaker_count ?? 2}{" "}
            {(draft.default_speaker_count ?? 2) === 1 ? "speaker" : "speakers"}
            {draft.use_medical_for_english ? " · Medical" : ""}
            {draft.translate_to_english ? " · translate" : ""}
          </span>
        ) : undefined
      }
      defaultOpen
      contentClassName="border-t border-gray-100 p-4 space-y-4"
    >
        {loading ? (
          <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
            <Spinner />
            <span>Loading transcription preferences...</span>
          </div>
        ) : !draft ? (
          <ErrorBanner
            message={error ?? "Could not load preferences."}
            className=""
          />
        ) : (
          <>
            <p className="text-xs text-gray-500">
              Applied to audio files dropped onto a client record. The "Upload
              audio file…" wizard uses these as starting values and lets you
              override per file.
            </p>

            {/* Language */}
            <fieldset>
              <legend className="text-sm font-medium text-gray-700 mb-2">
                Default language
              </legend>
              <div className="space-y-1.5">
                {(["english", "spanish", "mixed"] as TranscriptionLanguage[]).map(
                  (lang) => (
                    <label
                      key={lang}
                      className="flex items-start gap-2.5 cursor-pointer"
                    >
                      <input
                        type="radio"
                        name="default-language"
                        checked={draft.default_language === lang}
                        onChange={() => handleLanguageChange(lang)}
                        className="mt-0.5"
                      />
                      <div>
                        <span className="text-sm text-gray-900">
                          {labelForLanguage(lang)}
                        </span>
                        <p className="text-xs text-gray-500">
                          {descriptionForLanguage(lang)}
                        </p>
                      </div>
                    </label>
                  )
                )}
              </div>
            </fieldset>

            {/* Speakers */}
            <div>
              <label className="text-sm font-medium text-gray-700 block mb-1.5">
                Default speakers
              </label>
              <select
                value={draft.default_speaker_count}
                onChange={(e) => handleSpeakerCountChange(Number(e.target.value))}
                className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
              >
                <option value={1}>1 — single speaker (no diarization)</option>
                <option value={2}>2 — typical clinician + patient</option>
                <option value={3}>3 — small group</option>
                <option value={4}>4 — family or panel</option>
              </select>
              <p className="text-xs text-gray-500 mt-1">
                Picking "1" turns diarization off and produces a single text
                block (cheaper, faster).
              </p>
            </div>

            {/* Medical toggle */}
            <label className="flex items-start gap-2.5 cursor-pointer">
              <input
                type="checkbox"
                checked={draft.use_medical_for_english}
                onChange={(e) => handleMedicalToggle(e.target.checked)}
                className="mt-0.5"
              />
              <div>
                <span className="text-sm text-gray-900">
                  Use Transcribe Medical for English sessions
                </span>
                <p className="text-xs text-gray-500">
                  $0.075/min vs Standard $0.024/min — better recognition of
                  clinical vocabulary, drug names, and PHI tagging. Spanish and
                  Mixed sessions always use Standard.
                </p>
              </div>
            </label>

            {/* Translation toggle */}
            <label className="flex items-start gap-2.5 cursor-pointer">
              <input
                type="checkbox"
                checked={draft.translate_to_english}
                onChange={(e) => handleTranslateToggle(e.target.checked)}
                className="mt-0.5"
              />
              <div>
                <span className="text-sm text-gray-900">
                  Translate non-English segments to English
                </span>
                <p className="text-xs text-gray-500">
                  When a segment's detected language isn't English, render an
                  English translation alongside the original using your
                  preferred chat model. Adds a few cents per session.
                </p>
              </div>
            </label>

            {syncError ? (
              <ErrorBanner
                message={`Could not save transcription preferences: ${syncError}`}
                onRetry={() => {
                  if (draft) void sync(draft);
                }}
                className=""
              />
            ) : syncing ? (
              <p className="text-xs text-gray-400 pt-2 border-t border-gray-100 flex items-center gap-1.5">
                <Spinner /> Saving...
              </p>
            ) : dirty ? (
              <p className="text-xs text-gray-400 pt-2 border-t border-gray-100">
                Unsaved changes...
              </p>
            ) : syncedOnce ? (
              <p className="text-xs text-gray-400 pt-2 border-t border-gray-100">
                Saved. Other computers pick this up on restart.
              </p>
            ) : null}
          </>
        )}
    </PreferencesSection>
  );
}

function labelForLanguage(lang: TranscriptionLanguage): string {
  switch (lang) {
    case "english":
      return "English";
    case "spanish":
      return "Spanish";
    case "mixed":
      return "Mixed (interpreter)";
  }
}

function descriptionForLanguage(lang: TranscriptionLanguage): string {
  switch (lang) {
    case "english":
      return "All English audio. Most common.";
    case "spanish":
      return "All Spanish audio. Always Standard engine.";
    case "mixed":
      return "English and Spanish interleaved. Standard engine, no PHI tagging.";
  }
}
