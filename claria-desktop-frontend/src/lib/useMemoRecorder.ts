import { useEffect, useRef, useState } from "react";
import {
  createTextRecordFile,
  getWhisperModels,
  transcribeMemo,
  type WhisperModelInfo,
} from "./tauri";
import { float32ToBase64, mergePcmChunks, resampleTo16kHz } from "./audio";

export type MemoState =
  | "idle"
  | "recording"
  | "paused"
  | "transcribing"
  | "review";

/**
 * The live-memo capture engine.
 *
 * Owns the microphone, the `AudioContext` that taps it, the growing PCM
 * buffer, and the two intervals that drive the UI (a 1 s clock and a 4 s
 * re-transcription pass). None of that belongs in a page component: the
 * buffer and the context are refs precisely because touching them must not
 * re-render, and the state machine below is the only thing that should ever
 * see them.
 *
 * The transcript is deliberately re-derived from the *whole* buffer on every
 * pass rather than appended to. Whisper's output for a longer window can
 * revise earlier words, so a running concatenation would freeze mistakes in
 * place; re-running the full buffer costs more but reads correctly, and the
 * user only ever sees the latest full pass.
 *
 * Errors are reported through `onError` rather than swallowed — a failed
 * transcription pass must surface, or the user watches an empty transcript
 * and assumes the microphone is dead.
 */
export function useMemoRecorder({
  clientId,
  onError,
  onSaved,
}: {
  clientId: string;
  /** Report a failure, or clear the caller's banner with `null`. */
  onError: (message: string | null) => void;
  /** Called after a memo is written, so the caller can refresh its file list. */
  onSaved: () => Promise<void> | void;
}) {
  // Whether a Whisper model is installed, plus what it can do. Read once.
  const [ready, setReady] = useState(false);
  const [multilingual, setMultilingual] = useState(false);
  const [gpu, setGpu] = useState(false);
  const [modelLabel, setModelLabel] = useState("");

  const [state, setState] = useState<MemoState>("idle");
  const [transcript, setTranscript] = useState("");
  const [elapsed, setElapsed] = useState(0);
  const [filename, setFilename] = useState("");
  const [saving, setSaving] = useState(false);
  const [detectedLanguage, setDetectedLanguage] = useState<string | null>(null);

  // Audio capture handles (not state — no re-renders needed).
  const audioCtxRef = useRef<AudioContext | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const pcmBufferRef = useRef<Float32Array[]>([]);
  const transcribeTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const elapsedTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const transcribingRef = useRef(false);

  // The interval callbacks outlive the render that created them, so the
  // caller's callbacks are read through refs rather than captured.
  const onErrorRef = useRef(onError);
  const onSavedRef = useRef(onSaved);
  useEffect(() => {
    onErrorRef.current = onError;
    onSavedRef.current = onSaved;
  });

  // Check if a Whisper model is active. `cancelled` keeps a slow round-trip
  // from writing into a recorder that has already gone away.
  useEffect(() => {
    let cancelled = false;
    getWhisperModels()
      .then((models: WhisperModelInfo[]) => {
        if (cancelled) return;
        const active = models.find((m) => m.active);
        setReady(!!active);
        setMultilingual(active ? active.tier !== "base_en" : false);
        setGpu(active ? active.gpu_accelerated : false);
        setModelLabel(active ? active.dir_name : "");
      })
      .catch(() => {
        if (!cancelled) setReady(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

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
      const raw = mergePcmChunks(pcmBufferRef.current);
      if (raw.length === 0) return;
      const pcm16k = await resampleTo16kHz(raw, sampleRate);
      const base64 = float32ToBase64(pcm16k);
      const result = await transcribeMemo(base64);
      setTranscript(result.text);
      if (result.language) {
        setDetectedLanguage(result.language);
      }
    } catch (e) {
      console.error("Transcription error:", e);
      onErrorRef.current(String(e));
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
      setElapsed((prev) => prev + 1);
    }, 1000);
  }

  function stopElapsedTimer() {
    if (elapsedTimerRef.current) {
      clearInterval(elapsedTimerRef.current);
      elapsedTimerRef.current = null;
    }
  }

  async function start() {
    onErrorRef.current(null);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      streamRef.current = stream;

      const ctx = new AudioContext();
      audioCtxRef.current = ctx;
      pcmBufferRef.current = [];
      setTranscript("");
      setElapsed(0);
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

      setState("recording");
      startElapsedTimer();
      startTranscribeTimer();
    } catch (e) {
      onErrorRef.current(String(e));
    }
  }

  async function pause() {
    stopTranscribeTimer();
    stopElapsedTimer();
    if (audioCtxRef.current && audioCtxRef.current.state === "running") {
      await audioCtxRef.current.suspend();
    }
    // Run one final transcription pass before allowing edits.
    await runTranscription(true);
    setState("paused");
  }

  async function resume() {
    if (audioCtxRef.current && audioCtxRef.current.state === "suspended") {
      await audioCtxRef.current.resume();
    }
    setState("recording");
    startElapsedTimer();
    startTranscribeTimer();
  }

  async function done() {
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
    setState("transcribing");
    await runTranscription(true);

    // Close AudioContext.
    if (audioCtxRef.current && audioCtxRef.current.state !== "closed") {
      await audioCtxRef.current.close();
    }
    audioCtxRef.current = null;

    setFilename(`memo-${defaultMemoStamp(new Date())}`);
    setState("review");
  }

  function cancel() {
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
    setState("idle");
    setTranscript("");
    setElapsed(0);
    setDetectedLanguage(null);
  }

  async function save() {
    if (!filename.trim()) return;
    setSaving(true);
    onErrorRef.current(null);
    try {
      await createTextRecordFile(clientId, filename.trim(), transcript);
      pcmBufferRef.current = [];
      setState("idle");
      setTranscript("");
      setElapsed(0);
      setFilename("");
      setDetectedLanguage(null);
      await onSavedRef.current();
    } catch (e) {
      onErrorRef.current(String(e));
    } finally {
      setSaving(false);
    }
  }

  return {
    ready,
    multilingual,
    gpu,
    modelLabel,
    state,
    transcript,
    setTranscript,
    elapsed,
    filename,
    setFilename,
    saving,
    detectedLanguage,
    start,
    pause,
    resume,
    done,
    cancel,
    save,
  };
}

/** `YYYYMMDD-HHMM` stamp used to seed the review modal's filename. */
export function defaultMemoStamp(now: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}` +
    `-${pad(now.getHours())}${pad(now.getMinutes())}`
  );
}
