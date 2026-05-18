import { useEffect, useState } from "react";
import {
  fetchCloudPreferences,
  loadConfig,
  pickAudioFile,
  uploadRecordFileWithOptions,
  type ConfigInfo,
  type SpeakerMode,
  type TranscribeOptionsOverrides,
  type TranscriptionLanguage,
} from "../lib/tauri";

/**
 * Per-file transcription wizard.
 *
 * Surfaces all the per-job knobs (language, speaker handling, Medical engine,
 * translation) on top of the user's saved preferences. The wizard owns the
 * file picker too — drag-and-drop on the records page is the fast path; this
 * is the configure-then-upload path. No drag target inside the wizard (avoids
 * the geometry-sensitive UI flagged by the low-dexterity feedback memo).
 */
export default function TranscribeWizard({
  clientId,
  onClose,
  onUploaded,
}: {
  clientId: string;
  onClose: () => void;
  onUploaded: () => void;
}) {
  const [snapshot, setSnapshot] = useState<ConfigInfo | null>(null);
  const [loadingPrefs, setLoadingPrefs] = useState(true);
  const [prefsError, setPrefsError] = useState<string | null>(null);

  // The wizard's editable per-file overrides. Each null means "use the
  // saved preference for this field." We send a fully-populated overrides
  // object to the backend so behaviour is unambiguous on the Rust side.
  const [filePath, setFilePath] = useState<string | null>(null);
  const [language, setLanguage] = useState<TranscriptionLanguage | null>(null);
  const [speakerCount, setSpeakerCount] = useState<number | null>(null);
  const [speakerMode, setSpeakerMode] = useState<SpeakerMode | null>(null);
  const [useMedical, setUseMedical] = useState<boolean | null>(null);
  const [translate, setTranslate] = useState<boolean | null>(null);

  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);

  // Pull the latest synced prefs on mount so the wizard's pre-filled values
  // match what the user expects.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const info = await fetchCloudPreferences();
        if (cancelled) return;
        setSnapshot(info);
      } catch {
        try {
          const info = await loadConfig();
          if (cancelled) return;
          setSnapshot(info);
        } catch (e) {
          if (cancelled) return;
          setPrefsError(String(e));
        }
      } finally {
        if (!cancelled) setLoadingPrefs(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const effectiveLanguage: TranscriptionLanguage =
    language ?? snapshot?.transcription.default_language ?? "english";
  const effectiveSpeakerCount =
    speakerCount ?? snapshot?.transcription.default_speaker_count ?? 2;
  const effectiveUseMedical =
    useMedical ?? snapshot?.transcription.use_medical_for_english ?? false;
  const effectiveTranslate =
    translate ?? snapshot?.transcription.translate_to_english ?? false;

  // The wizard's engine line is derived. Medical only fires when language is
  // English and the user has it enabled; otherwise we always end up on
  // Standard. Showing it lets power users see what'll actually run.
  const engineSummary =
    effectiveLanguage === "english" && effectiveUseMedical
      ? "Transcribe Medical (en-US, $0.075/min, PHI tagging)"
      : effectiveLanguage === "mixed"
      ? "Transcribe Standard (en-US + es-US, auto-detect)"
      : effectiveLanguage === "spanish"
      ? "Transcribe Standard (es-US, $0.024/min)"
      : "Transcribe Standard (en-US, $0.024/min)";

  async function handlePickFile() {
    setUploadError(null);
    try {
      const picked = await pickAudioFile();
      if (picked) setFilePath(picked);
    } catch (e) {
      setUploadError(String(e));
    }
  }

  async function handleStart() {
    if (!filePath) return;
    setUploading(true);
    setUploadError(null);
    try {
      const overrides: TranscribeOptionsOverrides = {
        language,
        speaker_mode: speakerMode,
        speaker_count: speakerCount,
        use_medical_for_english: useMedical,
        translate_to_english: translate,
      };
      await uploadRecordFileWithOptions(clientId, filePath, overrides);
      onUploaded();
      onClose();
    } catch (e) {
      setUploadError(String(e));
    } finally {
      setUploading(false);
    }
  }

  const filename = filePath ? filePath.split("/").pop() ?? filePath : null;

  return (
    <div
      className="fixed inset-0 bg-black/40 z-50 flex items-center justify-center p-4"
      onClick={(e) => {
        if (e.target === e.currentTarget && !uploading) onClose();
      }}
    >
      <div className="bg-white rounded-lg shadow-xl max-w-lg w-full max-h-[90vh] overflow-y-auto">
        <div className="px-5 py-4 border-b border-gray-200 flex items-center justify-between">
          <h3 className="font-semibold text-gray-900">Upload audio file</h3>
          <button
            onClick={onClose}
            disabled={uploading}
            className="text-gray-400 hover:text-gray-600 disabled:opacity-50"
            aria-label="Close"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="px-5 py-4 space-y-5">
          {loadingPrefs ? (
            <p className="text-sm text-gray-500">Loading your defaults...</p>
          ) : prefsError ? (
            <div className="bg-red-50 border border-red-200 rounded-lg p-3">
              <p className="text-red-800 text-sm">{prefsError}</p>
            </div>
          ) : (
            <>
              {/* File picker */}
              <div>
                <label className="text-sm font-medium text-gray-700 block mb-1.5">
                  Audio file
                </label>
                {filename ? (
                  <div className="flex items-center justify-between gap-3 px-3 py-2 border border-gray-300 rounded-lg bg-gray-50">
                    <span className="text-sm text-gray-900 truncate">{filename}</span>
                    <button
                      onClick={handlePickFile}
                      disabled={uploading}
                      className="text-xs text-blue-600 hover:underline disabled:opacity-50"
                    >
                      Change
                    </button>
                  </div>
                ) : (
                  <button
                    onClick={handlePickFile}
                    className="w-full px-3 py-2 text-sm text-blue-600 border border-dashed border-blue-300 rounded-lg hover:bg-blue-50 transition-colors"
                  >
                    Choose a file…
                  </button>
                )}
                <p className="text-xs text-gray-500 mt-1">
                  Supports MP3, M4A, WAV, FLAC, OGG, AMR, WebM. Max 4 hours / 2 GB.
                </p>
              </div>

              {/* Language */}
              <fieldset>
                <legend className="text-sm font-medium text-gray-700 mb-2">Language</legend>
                <div className="space-y-1.5">
                  {(["english", "spanish", "mixed"] as TranscriptionLanguage[]).map((lang) => (
                    <label key={lang} className="flex items-start gap-2.5 cursor-pointer">
                      <input
                        type="radio"
                        name="wizard-language"
                        checked={effectiveLanguage === lang}
                        onChange={() => setLanguage(lang)}
                        className="mt-0.5"
                      />
                      <div>
                        <span className="text-sm text-gray-900">
                          {languageLabel(lang)}
                        </span>
                        <p className="text-xs text-gray-500">
                          {languageDescription(lang)}
                        </p>
                      </div>
                    </label>
                  ))}
                </div>
              </fieldset>

              {/* Speaker handling */}
              <div>
                <label className="text-sm font-medium text-gray-700 block mb-1.5">
                  Speakers
                </label>
                <select
                  value={
                    speakerMode === "channels"
                      ? "channels"
                      : String(effectiveSpeakerCount)
                  }
                  onChange={(e) => {
                    const v = e.target.value;
                    if (v === "channels") {
                      setSpeakerMode("channels");
                      setSpeakerCount(null);
                    } else {
                      const n = Number(v);
                      setSpeakerMode(n <= 1 ? "none" : "diarize");
                      setSpeakerCount(n);
                    }
                  }}
                  className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                >
                  <option value="1">1 — single speaker (no diarization)</option>
                  <option value="2">2 — clinician + patient (typical)</option>
                  <option value="3">3 — small group</option>
                  <option value="4">4 — family or panel</option>
                  <option value="channels">
                    Stereo channels (clinician L, patient R)
                  </option>
                </select>
              </div>

              {/* Medical override (English only) */}
              {effectiveLanguage === "english" && (
                <label className="flex items-start gap-2.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={effectiveUseMedical}
                    onChange={(e) => setUseMedical(e.target.checked)}
                    className="mt-0.5"
                  />
                  <div>
                    <span className="text-sm text-gray-900">
                      Transcribe Medical (this file only)
                    </span>
                    <p className="text-xs text-gray-500">
                      Clinical vocabulary + PHI tagging. $0.075/min vs $0.024/min Standard.
                    </p>
                  </div>
                </label>
              )}

              {/* Translation toggle */}
              <label className="flex items-start gap-2.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={effectiveTranslate}
                  onChange={(e) => setTranslate(e.target.checked)}
                  className="mt-0.5"
                />
                <div>
                  <span className="text-sm text-gray-900">
                    Translate non-English segments to English
                  </span>
                  <p className="text-xs text-gray-500">
                    Adds a translation alongside the original. Costs a few cents
                    per session via your preferred chat model.
                  </p>
                </div>
              </label>

              {/* Engine summary */}
              <div className="bg-gray-50 border border-gray-200 rounded-lg p-3">
                <p className="text-xs text-gray-500 mb-0.5">Will use:</p>
                <p className="text-sm text-gray-900">{engineSummary}</p>
              </div>

              {uploadError && (
                <div className="bg-red-50 border border-red-200 rounded-lg p-3">
                  <p className="text-red-800 text-sm">{uploadError}</p>
                </div>
              )}
            </>
          )}
        </div>

        <div className="px-5 py-3 border-t border-gray-200 flex items-center justify-end gap-2">
          <button
            onClick={onClose}
            disabled={uploading}
            className="px-3 py-1.5 text-sm text-gray-700 hover:bg-gray-100 rounded-lg disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            onClick={handleStart}
            disabled={!filePath || uploading || loadingPrefs}
            className="px-3 py-1.5 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {uploading ? "Transcribing..." : "Start"}
          </button>
        </div>
      </div>
    </div>
  );
}

function languageLabel(lang: TranscriptionLanguage): string {
  switch (lang) {
    case "english":
      return "English";
    case "spanish":
      return "Spanish";
    case "mixed":
      return "Mixed (interpreter session)";
  }
}

function languageDescription(lang: TranscriptionLanguage): string {
  switch (lang) {
    case "english":
      return "All English audio.";
    case "spanish":
      return "All Spanish audio. Always Standard engine.";
    case "mixed":
      return "English and Spanish interleaved. Standard engine, no PHI tagging.";
  }
}
