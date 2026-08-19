import { useCallback, useEffect, useRef, useState } from "react";
import { listWriterLibraryPrompts, type WriterPrompt } from "./tauri";

/**
 * The saved writer-prompt library, shared by the Writing page pickers and
 * the Preferences prompt manager so both surfaces load it the same way.
 *
 * `setPrompts` is exposed for callers that mutate the library (save,
 * delete) and want to update the list without a refetch.
 */
export function useWriterPrompts() {
  const [prompts, setPrompts] = useState<WriterPrompt[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const generationRef = useRef(0);

  const reload = useCallback(async () => {
    const generation = ++generationRef.current;
    setLoading(true);
    setError(null);
    try {
      const result = await listWriterLibraryPrompts();
      if (generation !== generationRef.current) return;
      setPrompts(result);
    } catch (reason) {
      if (generation !== generationRef.current) return;
      setError(String(reason));
    } finally {
      if (generation === generationRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
    return () => {
      generationRef.current += 1;
    };
  }, [reload]);

  return { prompts, setPrompts, loading, error, reload };
}
