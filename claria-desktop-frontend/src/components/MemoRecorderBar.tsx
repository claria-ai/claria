import Spinner from "./Spinner";
import { formatElapsed } from "../lib/audio";
import type { MemoState } from "../lib/useMemoRecorder";

/**
 * The strip that replaces the file-list header while a memo is being
 * recorded: status light, clock, engine badge, transport buttons, and the
 * live transcript.
 *
 * Renders nothing outside the three in-progress states — `review` is the
 * modal's job and `idle` has no bar at all.
 *
 * While paused the transcript becomes an editable textarea. That is the one
 * point in the flow where the user can correct local inference before the next pass
 * overwrites the text, so the edit target must be a real input, not a `<pre>`.
 */
export default function MemoRecorderBar({
  state,
  elapsed,
  transcript,
  onTranscriptChange,
  multilingual,
  detectedLanguage,
  gpu,
  modelLabel,
  onPause,
  onResume,
  onDone,
  onCancel,
}: {
  state: MemoState;
  elapsed: number;
  transcript: string;
  onTranscriptChange: (text: string) => void;
  /** Whether the active model can detect a language other than English. */
  multilingual: boolean;
  detectedLanguage: string | null;
  gpu: boolean;
  modelLabel: string;
  onPause: () => void;
  onResume: () => void;
  onDone: () => void;
  onCancel: () => void;
}) {
  if (state !== "recording" && state !== "paused" && state !== "transcribing") {
    return null;
  }

  const awaitingLanguage =
    multilingual && state === "recording" && !detectedLanguage;

  return (
    <div className="px-4 py-3 border-b border-gray-100 bg-red-50">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          {state === "recording" && (
            <span className="relative flex h-3 w-3">
              <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-red-400 opacity-75" />
              <span className="relative inline-flex rounded-full h-3 w-3 bg-red-500" />
            </span>
          )}
          {state === "paused" && (
            <span className="inline-flex rounded-full h-3 w-3 bg-yellow-500" />
          )}
          {state === "transcribing" && <Spinner />}
          <span className="text-sm font-medium text-gray-700">
            {state === "recording" && "Recording"}
            {state === "paused" && "Paused"}
            {state === "transcribing" && "Transcribing..."}
          </span>
          <span className="text-sm text-gray-500 font-mono">
            {formatElapsed(elapsed)}
          </span>
          <span
            className={`px-1.5 py-0.5 text-xs rounded ${
              gpu ? "bg-green-100 text-green-700" : "bg-gray-100 text-gray-500"
            }`}
          >
            {gpu ? "GPU" : "CPU"}
          </span>
          {modelLabel && (
            <span
              title={`Local model: ${modelLabel}${gpu ? " (accelerated)" : " (CPU)"}`}
              className="inline-flex items-center justify-center w-4 h-4 text-xs text-gray-400 border border-gray-300 rounded-full cursor-help hover:text-gray-600 hover:border-gray-400 transition-colors"
            >
              ?
            </span>
          )}
        </div>
        <div className="flex gap-2">
          {state === "recording" && (
            <>
              <button
                onClick={onPause}
                className="px-3 py-1 text-xs font-medium text-yellow-700 bg-yellow-100 border border-yellow-300 rounded hover:bg-yellow-200 transition-colors"
              >
                Pause
              </button>
              <button
                onClick={onDone}
                className="px-3 py-1 text-xs font-medium text-gray-700 bg-gray-100 border border-gray-300 rounded hover:bg-gray-200 transition-colors"
              >
                Done
              </button>
            </>
          )}
          {state === "paused" && (
            <>
              <button
                onClick={onResume}
                className="px-3 py-1 text-xs font-medium text-red-700 bg-red-100 border border-red-300 rounded hover:bg-red-200 transition-colors"
              >
                Resume
              </button>
              <button
                onClick={onDone}
                className="px-3 py-1 text-xs font-medium text-gray-700 bg-gray-100 border border-gray-300 rounded hover:bg-gray-200 transition-colors"
              >
                Done
              </button>
            </>
          )}
          <button
            onClick={onCancel}
            className="px-3 py-1 text-xs font-medium text-gray-500 hover:text-gray-700 transition-colors"
            disabled={state === "transcribing"}
          >
            Cancel
          </button>
        </div>
      </div>

      {/* Live transcript */}
      {(transcript || awaitingLanguage) && (
        <div className="mt-3">
          {detectedLanguage && (
            <span className="inline-block px-1.5 py-0.5 text-xs font-medium bg-blue-100 text-blue-700 rounded mb-1.5">
              {detectedLanguage.toUpperCase()}
            </span>
          )}
          {awaitingLanguage && !transcript && (
            <p className="text-xs text-gray-400 italic py-2">
              Detecting language...
            </p>
          )}
          {state === "paused" ? (
            <textarea
              value={transcript}
              onChange={(e) => onTranscriptChange(e.target.value)}
              className="w-full min-h-[100px] px-3 py-2 text-sm font-mono border border-gray-300 rounded-lg resize-y focus:outline-none focus:ring-2 focus:ring-yellow-500 focus:border-transparent"
            />
          ) : transcript ? (
            <pre className="text-sm text-gray-700 whitespace-pre-wrap font-mono bg-white border border-gray-200 rounded-lg p-3 max-h-[200px] overflow-y-auto">
              {transcript}
            </pre>
          ) : null}
        </div>
      )}
    </div>
  );
}
