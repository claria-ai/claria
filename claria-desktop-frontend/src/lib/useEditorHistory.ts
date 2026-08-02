import { useEffect, useState } from "react";
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
      .catch(() => {
        if (!cancelled) setEntries([]);
      });

    return () => {
      cancelled = true;
    };
  }, [clientId]);

  return entries;
}
