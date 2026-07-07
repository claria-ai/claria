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
  getWhisperModels,
  transcribeMemo,
  loadConfig,
  type RecordFile,
  type ChatHistoryDetail,
  type ChatModel,
  type FileVersion,
  type DeletedFile,
  type TranscriptionPreferences,
  type WhisperModelInfo,
} from "../lib/tauri";
import TranscribeWizard from "../components/TranscribeWizard";
import TranscriptEditor from "../components/TranscriptEditor";
import Spinner from "../components/Spinner";
import MoreToggle from "../components/MoreToggle";
import DeletedSection from "../components/DeletedSection";
import { ErrorBanner } from "../components/StateCards";
import { formatDateTime, formatFileSize } from "../lib/format";
import { useMoreMode } from "../lib/useMoreMode";
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

  // Memo recording state
  const [memoReady, setMemoReady] = useState(false);
  const [memoMultilingual, setMemoMultilingual] = useState(false);
  const [memoGpu, setMemoGpu] = useState(false);
  const [memoModelLabel, setMemoModelLabel] = useState("");
  type MemoState = "idle" | "recording" | "paused" | "transcribing" | "review";
  const [memoState, setMemoState] = useState<MemoState>("idle");
  const [memoTranscript, setMemoTranscript] = useState("");
  const [memoElapsed, setMemoElapsed] = useState(0);
  const [memoFilename, setMemoFilename] = useState("");
  const [memoSaving, setMemoSaving] = useState(false);
  const [detectedLanguage, setDetectedLanguage] = useState<string | null>(null);

  // Audio capture refs (not state — no re-renders needed)
  const audioCtxRef = useRef<AudioContext | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const pcmBufferRef = useRef<Float32Array[]>([]);
  const transcribeTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const elapsedTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const transcribingRef = useRef(false);

  const CHAT_HISTORY_PREFIX = "chat-history/";

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

  const query = search.trim().toLowerCase();
  const chatHistoryFiles = files
    .filter((f) => f.filename.startsWith(CHAT_HISTORY_PREFIX))
    .sort((a, b) => (b.uploaded_at ?? "").localeCompare(a.uploaded_at ?? ""));
  const regularFiles = files.filter(
    (f) =>
      !f.filename.startsWith(CHAT_HISTORY_PREFIX) &&
      (f.filename.toLowerCase().includes(query) ||
        contentMatches.has(f.filename)),
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

  // Check if a Whisper model is active.
  useEffect(() => {
    getWhisperModels()
      .then((models: WhisperModelInfo[]) => {
        const active = models.find((m) => m.active);
        setMemoReady(!!active);
        setMemoMultilingual(active ? active.tier !== "base_en" : false);
        setMemoGpu(active ? active.gpu_accelerated : false);
        setMemoModelLabel(active ? active.dir_name : "");
      })
      .catch(() => setMemoReady(false));
  }, []);

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

  // -------------------------------------------------------------------------
  // Memo recording helpers
  // -------------------------------------------------------------------------

  function getPcmBuffer(): Float32Array {
    const chunks = pcmBufferRef.current;
    const totalLen = chunks.reduce((sum, c) => sum + c.length, 0);
    const merged = new Float32Array(totalLen);
    let offset = 0;
    for (const chunk of chunks) {
      merged.set(chunk, offset);
      offset += chunk.length;
    }
    return merged;
  }

  async function resampleTo16kHz(buffer: Float32Array, sourceSampleRate: number): Promise<Float32Array> {
    if (sourceSampleRate === 16000) return buffer;
    const duration = buffer.length / sourceSampleRate;
    const offlineCtx = new OfflineAudioContext(1, Math.ceil(duration * 16000), 16000);
    const audioBuffer = offlineCtx.createBuffer(1, buffer.length, sourceSampleRate);
    audioBuffer.getChannelData(0).set(buffer);
    const source = offlineCtx.createBufferSource();
    source.buffer = audioBuffer;
    source.connect(offlineCtx.destination);
    source.start();
    const rendered = await offlineCtx.startRendering();
    return rendered.getChannelData(0);
  }

  function float32ToBase64(samples: Float32Array): string {
    const bytes = new Uint8Array(samples.buffer, samples.byteOffset, samples.byteLength);
    let binary = "";
    for (let i = 0; i < bytes.length; i++) {
      binary += String.fromCharCode(bytes[i]);
    }
    return btoa(binary);
  }

  async function runTranscription(force = false) {
    if (!force && transcribingRef.current) return;
    // If forced (final pass), wait for any in-flight transcription to finish.
    if (force) {
      while (transcribingRef.current) {
        await new Promise((r) => setTimeout(r, 100));
      }
    }
    transcribingRef.current = true;
    try {
      const sampleRate = audioCtxRef.current?.sampleRate ?? 44100;
      const raw = getPcmBuffer();
      if (raw.length === 0) return;
      const pcm16k = await resampleTo16kHz(raw, sampleRate);
      const base64 = float32ToBase64(pcm16k);
      const result = await transcribeMemo(base64);
      setMemoTranscript(result.text);
      if (result.language) {
        setDetectedLanguage(result.language);
      }
    } catch (e) {
      console.error("Transcription error:", e);
      setError(String(e));
    } finally {
      transcribingRef.current = false;
    }
  }

  function startTranscribeTimer() {
    if (transcribeTimerRef.current) return;
    transcribeTimerRef.current = setInterval(() => {
      runTranscription();
    }, 4000);
  }

  function stopTranscribeTimer() {
    if (transcribeTimerRef.current) {
      clearInterval(transcribeTimerRef.current);
      transcribeTimerRef.current = null;
    }
  }

  function startElapsedTimer() {
    if (elapsedTimerRef.current) return;
    elapsedTimerRef.current = setInterval(() => {
      setMemoElapsed((prev) => prev + 1);
    }, 1000);
  }

  function stopElapsedTimer() {
    if (elapsedTimerRef.current) {
      clearInterval(elapsedTimerRef.current);
      elapsedTimerRef.current = null;
    }
  }

  async function handleStartMemo() {
    setError(null);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      streamRef.current = stream;

      const ctx = new AudioContext();
      audioCtxRef.current = ctx;
      pcmBufferRef.current = [];
      setMemoTranscript("");
      setMemoElapsed(0);
      setDetectedLanguage(null);

      const source = ctx.createMediaStreamSource(stream);
      // ScriptProcessorNode is deprecated but widely supported and simpler
      // than AudioWorklet for this use case.
      const processor = ctx.createScriptProcessor(4096, 1, 1);
      processor.onaudioprocess = (e) => {
        const input = e.inputBuffer.getChannelData(0);
        pcmBufferRef.current.push(new Float32Array(input));
      };
      source.connect(processor);
      processor.connect(ctx.destination);

      setMemoState("recording");
      startElapsedTimer();
      startTranscribeTimer();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handlePauseMemo() {
    stopTranscribeTimer();
    stopElapsedTimer();
    if (audioCtxRef.current && audioCtxRef.current.state === "running") {
      await audioCtxRef.current.suspend();
    }
    // Run one final transcription pass before allowing edits.
    await runTranscription(true);
    setMemoState("paused");
  }

  async function handleResumeMemo() {
    if (audioCtxRef.current && audioCtxRef.current.state === "suspended") {
      await audioCtxRef.current.resume();
    }
    setMemoState("recording");
    startElapsedTimer();
    startTranscribeTimer();
  }

  async function handleDoneMemo() {
    stopTranscribeTimer();
    stopElapsedTimer();

    // Stop the media stream.
    streamRef.current?.getTracks().forEach((t) => t.stop());
    streamRef.current = null;

    if (audioCtxRef.current) {
      if (audioCtxRef.current.state !== "closed") {
        // Resume if suspended so we can run final transcription
        if (audioCtxRef.current.state === "suspended") {
          await audioCtxRef.current.resume();
        }
      }
    }

    // Final transcription pass.
    setMemoState("transcribing");
    await runTranscription(true);

    // Close AudioContext.
    if (audioCtxRef.current && audioCtxRef.current.state !== "closed") {
      await audioCtxRef.current.close();
    }
    audioCtxRef.current = null;

    const now = new Date();
    const ts = `${now.getFullYear()}${String(now.getMonth() + 1).padStart(2, "0")}${String(now.getDate()).padStart(2, "0")}-${String(now.getHours()).padStart(2, "0")}${String(now.getMinutes()).padStart(2, "0")}`;
    setMemoFilename(`memo-${ts}`);
    setMemoState("review");
  }

  function handleCancelMemo() {
    // Clean up any active audio resources.
    stopTranscribeTimer();
    stopElapsedTimer();
    streamRef.current?.getTracks().forEach((t) => t.stop());
    streamRef.current = null;
    if (audioCtxRef.current && audioCtxRef.current.state !== "closed") {
      audioCtxRef.current.close();
    }
    audioCtxRef.current = null;
    pcmBufferRef.current = [];
    setMemoState("idle");
    setMemoTranscript("");
    setMemoElapsed(0);
    setDetectedLanguage(null);
  }

  async function handleSaveMemo() {
    if (!memoFilename.trim()) return;
    setMemoSaving(true);
    setError(null);
    try {
      await createTextRecordFile(clientId, memoFilename.trim(), memoTranscript);
      pcmBufferRef.current = [];
      setMemoState("idle");
      setMemoTranscript("");
      setMemoElapsed(0);
      setMemoFilename("");
      setDetectedLanguage(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setMemoSaving(false);
    }
  }

  function formatElapsed(seconds: number): string {
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `${m}:${String(s).padStart(2, "0")}`;
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

  async function handleOpenVersions(filename: string) {
    setVersionFile(filename);
    setVersionsLoading(true);
    setVersionPreview(null);
    setSelectedVersions(new Set());
    setDiffResult(null);
    try {
      setVersions(await listFileVersions(clientId, versionKeyFor(filename)));
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
      const text = await getFileVersionText(
        clientId,
        versionKeyFor(versionFile!),
        versionId
      );
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
      const key = versionKeyFor(versionFile);
      const [text1, text2] = await Promise.all([
        getFileVersionText(clientId, key, v1),
        getFileVersionText(clientId, key, v2),
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
      await restoreFileVersion(clientId, versionKeyFor(versionFile), versionId);
      handleCloseVersions();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setRestoringVersion(false);
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
                    <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                    </svg>
                  </button>
                </>
              )}
              {!searchOpen && (
                <button
                  onClick={() => setSearchOpen(true)}
                  title="Search file names and contents"
                  className="p-1.5 rounded border border-gray-300 text-gray-400 hover:bg-gray-100 transition-colors"
                >
                  <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
                  </svg>
                </button>
              )}
              {!searchOpen && (
                <MoreToggle
                  active={moreMode}
                  onClick={toggleMoreMode}
                  title={moreMode ? "Hide version history" : "Show version history"}
                />
              )}
              {!searchOpen && memoReady && memoState === "idle" && (
                <button
                  onClick={handleStartMemo}
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

          {/* Recording bar */}
          {(memoState === "recording" || memoState === "paused" || memoState === "transcribing") && (
            <div className="px-4 py-3 border-b border-gray-100 bg-red-50">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  {memoState === "recording" && (
                    <span className="relative flex h-3 w-3">
                      <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-red-400 opacity-75" />
                      <span className="relative inline-flex rounded-full h-3 w-3 bg-red-500" />
                    </span>
                  )}
                  {memoState === "paused" && (
                    <span className="inline-flex rounded-full h-3 w-3 bg-yellow-500" />
                  )}
                  {memoState === "transcribing" && <Spinner />}
                  <span className="text-sm font-medium text-gray-700">
                    {memoState === "recording" && "Recording"}
                    {memoState === "paused" && "Paused"}
                    {memoState === "transcribing" && "Transcribing..."}
                  </span>
                  <span className="text-sm text-gray-500 font-mono">
                    {formatElapsed(memoElapsed)}
                  </span>
                  <span className={`px-1.5 py-0.5 text-xs rounded ${
                    memoGpu
                      ? "bg-green-100 text-green-700"
                      : "bg-gray-100 text-gray-500"
                  }`}>
                    {memoGpu ? "GPU" : "CPU"}
                  </span>
                  {memoModelLabel && (
                    <span
                      title={`Model: ${memoModelLabel}${memoGpu ? " (Metal GPU)" : " (CPU)"}`}
                      className="inline-flex items-center justify-center w-4 h-4 text-xs text-gray-400 border border-gray-300 rounded-full cursor-help hover:text-gray-600 hover:border-gray-400 transition-colors"
                    >
                      ?
                    </span>
                  )}
                </div>
                <div className="flex gap-2">
                  {memoState === "recording" && (
                    <>
                      <button
                        onClick={handlePauseMemo}
                        className="px-3 py-1 text-xs font-medium text-yellow-700 bg-yellow-100 border border-yellow-300 rounded hover:bg-yellow-200 transition-colors"
                      >
                        Pause
                      </button>
                      <button
                        onClick={handleDoneMemo}
                        className="px-3 py-1 text-xs font-medium text-gray-700 bg-gray-100 border border-gray-300 rounded hover:bg-gray-200 transition-colors"
                      >
                        Done
                      </button>
                    </>
                  )}
                  {memoState === "paused" && (
                    <>
                      <button
                        onClick={handleResumeMemo}
                        className="px-3 py-1 text-xs font-medium text-red-700 bg-red-100 border border-red-300 rounded hover:bg-red-200 transition-colors"
                      >
                        Resume
                      </button>
                      <button
                        onClick={handleDoneMemo}
                        className="px-3 py-1 text-xs font-medium text-gray-700 bg-gray-100 border border-gray-300 rounded hover:bg-gray-200 transition-colors"
                      >
                        Done
                      </button>
                    </>
                  )}
                  <button
                    onClick={handleCancelMemo}
                    className="px-3 py-1 text-xs font-medium text-gray-500 hover:text-gray-700 transition-colors"
                    disabled={memoState === "transcribing"}
                  >
                    Cancel
                  </button>
                </div>
              </div>

              {/* Live transcript */}
              {(memoTranscript || (memoMultilingual && memoState === "recording" && !detectedLanguage)) && (
                <div className="mt-3">
                  {detectedLanguage && (
                    <span className="inline-block px-1.5 py-0.5 text-xs font-medium bg-blue-100 text-blue-700 rounded mb-1.5">
                      {detectedLanguage.toUpperCase()}
                    </span>
                  )}
                  {memoMultilingual && memoState === "recording" && !detectedLanguage && !memoTranscript && (
                    <p className="text-xs text-gray-400 italic py-2">
                      Detecting language...
                    </p>
                  )}
                  {memoState === "paused" ? (
                    <textarea
                      value={memoTranscript}
                      onChange={(e) => setMemoTranscript(e.target.value)}
                      className="w-full min-h-[100px] px-3 py-2 text-sm font-mono border border-gray-300 rounded-lg resize-y focus:outline-none focus:ring-2 focus:ring-yellow-500 focus:border-transparent"
                    />
                  ) : memoTranscript ? (
                    <pre className="text-sm text-gray-700 whitespace-pre-wrap font-mono bg-white border border-gray-200 rounded-lg p-3 max-h-[200px] overflow-y-auto">
                      {memoTranscript}
                    </pre>
                  ) : null}
                </div>
              )}
            </div>
          )}

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
                    <div className="flex items-center gap-2 min-w-0">
                      <p className="text-sm font-medium text-gray-900 truncate">
                        {file.filename}
                      </p>
                      {query !== "" &&
                        contentMatches.has(file.filename) &&
                        !file.filename.toLowerCase().includes(query) && (
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
            onClose={() => {
              setPreviewText(null);
              setPreviewFilename(null);
            }}
            onSaved={(newBody) => setPreviewText(newBody)}
          />
        ) : (
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
        ))}

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
              its extracted text? You can restore this file again later if
              necessary.
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

      {/* Memo review modal */}
      {memoState === "review" && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-2xl w-full mx-4 p-6 max-h-[80vh] flex flex-col">
            <h3 className="text-lg font-semibold text-gray-900 mb-4">
              Review Memo
            </h3>
            <input
              type="text"
              placeholder="Filename (e.g. session-notes)"
              value={memoFilename}
              onChange={(e) => setMemoFilename(e.target.value)}
              className="w-full px-3 py-2 border border-gray-300 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent mb-3"
              autoFocus
            />
            <textarea
              value={memoTranscript}
              onChange={(e) => setMemoTranscript(e.target.value)}
              className="flex-1 min-h-[200px] w-full px-3 py-2 border border-gray-300 rounded-lg text-sm font-mono resize-none focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent mb-4"
            />
            <div className="flex justify-end gap-3">
              <button
                onClick={handleCancelMemo}
                className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
                disabled={memoSaving}
              >
                Discard
              </button>
              <button
                onClick={handleSaveMemo}
                disabled={memoSaving || !memoFilename.trim()}
                className="px-4 py-2 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {memoSaving ? "Saving..." : "Save"}
              </button>
            </div>
          </div>
        </div>
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

      {/* Version history modal */}
      {versionFile && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-4xl w-full mx-4 p-6 max-h-[90vh] flex flex-col">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-semibold text-gray-900">
                {isAudioSidecar(versionFile)
                  ? `Transcript History: ${versionFile}`
                  : `Version History: ${versionFile}`}
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
                            {v.last_modified ? formatDateTime(v.last_modified) : "Unknown date"}
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
                    <div className="border border-gray-200 rounded-lg overflow-auto max-h-[20rem]">
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

/**
 * True when the filename is a supported audio file (e.g. `session.m4a`).
 *
 * The preview modal is opened with the *audio* filename, not the sidecar
 * name — `getRecordFileText` handles the sidecar lookup internally, and
 * `save_transcript_edits` / `restore_original_transcript` also expect the
 * audio filename and append `.text` themselves. So the structured editor
 * gate is on the audio extension, not on a `.text` suffix.
 */
function isAudioSidecar(filename: string): boolean {
  const ext = filename.split(".").pop()?.toLowerCase() ?? "";
  return AUDIO_EXTENSIONS.has(ext);
}

/**
 * Map an audio filename to the S3 key that the version-history commands
 * should operate on. For audio files this is the `.text` sidecar — the
 * raw audio file's history isn't useful (it's a single immutable
 * upload). For everything else it's the file itself.
 */
function versionKeyFor(filename: string): string {
  return isAudioSidecar(filename) ? `${filename}.text` : filename;
}

function transcribeSummary(prefs: TranscriptionPreferences): string {
  const lang =
    prefs.default_language === "english"
      ? "English"
      : prefs.default_language === "spanish"
      ? "Spanish"
      : "Mixed (en+es)";
  const engine =
    prefs.default_language === "english" && prefs.use_medical_for_english
      ? "Medical"
      : "Standard";
  const speakers = `${prefs.default_speaker_count} speaker${
    (prefs.default_speaker_count ?? 0) === 1 ? "" : "s"
  }`;
  const translate = prefs.translate_to_english ? ", translate" : "";
  return `Audio uses: ${engine} · ${lang} · ${speakers}${translate}.`;
}

function transcribeTooltip(prefs: TranscriptionPreferences | null): string {
  if (!prefs) return "Drag files here to upload";
  // Rough ETA — Transcribe is typically ~5–10x real-time. Use 1 min processing
  // per 6 min of audio as the heuristic shown to users.
  return `${transcribeSummary(
    prefs
  )}\nDuration estimate: ~1 minute of processing per 6 minutes of audio.`;
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

