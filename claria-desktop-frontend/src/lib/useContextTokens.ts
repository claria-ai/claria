import { useEffect, useState } from "react";

// Shared lifecycle for the pre-flight context token count that every chat
// surface shows next to its "Context" label.
//
// The count is model-independent — the backend always counts against the
// newest available Haiku, because not every chat model supports the
// CountTokens API — so callers never supply a model and the count never
// re-runs when the user switches models.
//
// Pass `null` for `count` while there is nothing to count (context still
// loading, or no context at all); the hook then reports no count rather
// than a spinner. Results are tagged with the `count` that produced them,
// so a late reply from a superseded request is ignored.
export function useContextTokens(count: (() => Promise<number>) | null) {
  const [result, setResult] = useState<{
    source: (() => Promise<number>) | null;
    tokens: number | null;
    error: string | null;
  }>({ source: null, tokens: null, error: null });

  useEffect(() => {
    if (!count) return;
    let cancelled = false;
    count()
      .then((tokens) => {
        if (!cancelled) setResult({ source: count, tokens, error: null });
      })
      .catch((e) => {
        if (!cancelled)
          setResult({ source: count, tokens: null, error: String(e) });
      });
    return () => {
      cancelled = true;
    };
  }, [count]);

  const settled = count != null && result.source === count;
  return {
    tokens: settled ? result.tokens : null,
    error: settled ? result.error : null,
    counting: count != null && !settled,
  };
}
