import { useCallback, useEffect, useRef, useState } from "react";
import { listWriterTemplates, type WriterTemplateView } from "./tauri";

/**
 * The managed writer-template list, shared by the Writing page and the
 * Preferences template manager so both surfaces load it the same way.
 *
 * `setTemplates` is exposed for callers that mutate the shelf (upload,
 * rename, delete) and want to update the list without a refetch.
 */
export function useWriterTemplates() {
  const [templates, setTemplates] = useState<WriterTemplateView[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const generationRef = useRef(0);

  const reload = useCallback(async () => {
    const generation = ++generationRef.current;
    setLoading(true);
    setError(null);
    try {
      const result = await listWriterTemplates();
      if (generation !== generationRef.current) return;
      setTemplates(result);
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

  return { templates, setTemplates, loading, error, reload };
}
