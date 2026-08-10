import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type DependencyList,
} from "react";

/**
 * The blessed async-load lifecycle: run `load` on mount and whenever `deps`
 * change, expose `{ data, loading, error, reload }`, and ignore results from
 * superseded requests (staleness protection via a generation counter, modeled
 * on `useContextTokens`).
 *
 * Pass `null` for `load` while there is nothing to fetch yet; the hook then
 * reports not-loading and keeps any previous data.
 *
 * This replaces the hand-rolled `let cancelled = false` copies. Generation
 * counters inside feature hooks remain appropriate only where loads interleave
 * with mutations (e.g. the report workspace).
 */
export function useAsyncLoad<T>(
  load: (() => Promise<T>) | null,
  deps: DependencyList
): {
  data: T | null;
  loading: boolean;
  error: string | null;
  reload: () => void;
} {
  const [state, setState] = useState<{
    data: T | null;
    loading: boolean;
    error: string | null;
  }>({ data: null, loading: load != null, error: null });
  const generationRef = useRef(0);
  const loadRef = useRef(load);
  loadRef.current = load;

  const reload = useCallback(() => {
    const generation = ++generationRef.current;
    const fn = loadRef.current;
    if (!fn) {
      setState((prev) => (prev.loading ? { ...prev, loading: false } : prev));
      return;
    }
    setState((prev) => ({ ...prev, loading: true, error: null }));
    fn().then(
      (data) => {
        if (generation === generationRef.current) {
          setState({ data, loading: false, error: null });
        }
      },
      (reason) => {
        if (generation === generationRef.current) {
          setState((prev) => ({
            ...prev,
            loading: false,
            error: String(reason),
          }));
        }
      }
    );
    // The dependency list is the caller's contract, exactly like useEffect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  useEffect(() => {
    reload();
    return () => {
      generationRef.current += 1;
    };
  }, [reload]);

  return { ...state, reload };
}
