// Human-readable summaries of the transcription settings that will actually
// run. Two call sites render these: the record page's drop zone, which
// describes the saved defaults, and the transcribe wizard, which describes the
// per-file overrides the user has just picked.

import type {
  TranscriptionLanguage,
  TranscriptionPreferences,
} from "./tauri";

/**
 * Medical only fires when the language is English *and* the user has it
 * enabled; every other combination lands on Standard. Both summaries below
 * branch on this, so it lives in one place.
 */
export function usesMedicalEngine(
  language: TranscriptionLanguage | undefined,
  useMedicalForEnglish: boolean | undefined,
): boolean {
  return language === "english" && !!useMedicalForEnglish;
}

/** Engine plus dialect and price, as shown in the wizard's review step. */
export function transcribeEngineSummary(
  language: TranscriptionLanguage | undefined,
  useMedicalForEnglish: boolean | undefined,
): string {
  if (usesMedicalEngine(language, useMedicalForEnglish)) {
    return "Transcribe Medical (en-US, $0.075/min, PHI tagging)";
  }
  return language === "mixed"
    ? "Transcribe Standard (en-US + es-US, auto-detect)"
    : language === "spanish"
      ? "Transcribe Standard (es-US, $0.024/min)"
      : "Transcribe Standard (en-US, $0.024/min)";
}

/** One-line description of the saved defaults, shown under the drop zone. */
export function transcribeSummary(prefs: TranscriptionPreferences): string {
  const lang =
    prefs.default_language === "english"
      ? "English"
      : prefs.default_language === "spanish"
        ? "Spanish"
        : "Mixed (en+es)";
  const engine = usesMedicalEngine(
    prefs.default_language,
    prefs.use_medical_for_english,
  )
    ? "Medical"
    : "Standard";
  const speakers = `${prefs.default_speaker_count} speaker${
    (prefs.default_speaker_count ?? 0) === 1 ? "" : "s"
  }`;
  const translate = prefs.translate_to_english ? ", translate" : "";
  return `Audio uses: ${engine} · ${lang} · ${speakers}${translate}.`;
}

/** Drop-zone tooltip: the summary plus a rough processing-time heuristic. */
export function transcribeTooltip(
  prefs: TranscriptionPreferences | null,
): string {
  if (!prefs) return "Drag files here to upload";
  // Rough ETA — Transcribe is typically ~5–10x real-time. Use 1 min processing
  // per 6 min of audio as the heuristic shown to users.
  return `${transcribeSummary(
    prefs,
  )}\nDuration estimate: ~1 minute of processing per 6 minutes of audio.`;
}
