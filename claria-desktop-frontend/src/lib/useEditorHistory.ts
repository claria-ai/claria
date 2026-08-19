import { useEffect, useState } from "react";
import { logFrontendEvent } from "./logBridge";
import { listEditorHistory, type EditorHistoryEntry } from "./tauri";

/**
 * Persisted Writing sessions shown alongside a client's record files.
 *
 * History is additive to the Record workspace: an unavailable or corrupt
 * Writing session must not prevent the client's files from opening.
 */
export function useEditorHistory(clientId: string): EditorHistoryEntry[] {
  const [entries, setEntries] = useState<EditorHistoryEntry[]>([]);

  useEffect(() => {
    let cancelled = false;
    listEditorHistory(clientId)
      .then((history) => {
        if (!cancelled) setEntries(history);
      })
      .catch((reason) => {
        // Degrade to an empty folder, but never invisibly.
        logFrontendEvent("warn", `Editor history unavailable: ${reason}`);
        if (!cancelled) setEntries([]);
      });

    return () => {
      cancelled = true;
    };
  }, [clientId]);

  return entries;
}
