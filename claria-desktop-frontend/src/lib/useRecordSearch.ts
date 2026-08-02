import { useEffect, useRef, useState } from "react";
import { searchRecordContents } from "./tauri";

/**
 * Search over a record's files, by name and by extracted text.
 *
 * Names are matched locally on every keystroke so the list never lags the
 * input. Contents cost a round-trip, so that query is debounced and its
 * results arrive separately as a set of filenames — which is why a row can
 * match on contents alone and needs a badge to say so.
 */
export function useRecordSearch(
  clientId: string,
  onError: (message: string) => void,
) {
  const [search, setSearch] = useState("");
  // The search input replaces the header action buttons while open; closing
  // clears the query so files are never filtered by a hidden search box.
  const [searchOpen, setSearchOpen] = useState(false);
  const [debouncedSearch, setDebouncedSearch] = useState("");
  // Results for the last query that was actually sent. Kept across keystrokes
  // so the list doesn't flicker empty while the next round-trip is in flight.
  const [fetched, setFetched] = useState<Set<string>>(EMPTY);

  const onErrorRef = useRef(onError);
  useEffect(() => {
    onErrorRef.current = onError;
  });

  useEffect(() => {
    const t = setTimeout(() => setDebouncedSearch(search.trim()), 300);
    return () => clearTimeout(t);
  }, [search]);

  useEffect(() => {
    if (!debouncedSearch) return;
    let stale = false;
    searchRecordContents(clientId, debouncedSearch)
      .then((names) => {
        if (!stale) setFetched(new Set(names));
      })
      .catch((e) => {
        if (!stale) onErrorRef.current(String(e));
      });
    return () => {
      stale = true;
    };
  }, [clientId, debouncedSearch]);

  function close() {
    setSearch("");
    setSearchOpen(false);
  }

  return {
    search,
    setSearch,
    searchOpen,
    open: () => setSearchOpen(true),
    close,
    /** Trimmed query, for filtering and for the match-reason badge. */
    query: search.trim(),
    // Derived rather than cleared on the way down: with no query there is
    // nothing to match, and deriving it keeps the empty case out of an effect.
    contentMatches: debouncedSearch ? fetched : EMPTY,
  };
}

/** Shared empty set, so the no-query case keeps a stable identity. */
const EMPTY = new Set<string>();
