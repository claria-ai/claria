import { useEffect, useState } from "react";
import { loadConfig, type TranscriptionPreferences } from "./tauri";

/**
 * The saved transcription settings, read once.
 *
 * Only ever used to *describe* what a dropped audio file will get, so a
 * failure is swallowed: the caller simply shows no summary. `cancelled` keeps
 * a slow config read from landing on an unmounted page.
 */
export function useTranscriptionPrefs(): TranscriptionPreferences | null {
  const [prefs, setPrefs] = useState<TranscriptionPreferences | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const info = await loadConfig();
        if (!cancelled) setPrefs(info.transcription);
      } catch {
        // Silent — the tooltip just won't show prefs in that case.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return prefs;
}
