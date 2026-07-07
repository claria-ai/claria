// Accent-insensitive matching for type-ahead search fields.
//
// Both haystack and needle are folded, so "Luci" finds "Lucí" and typing
// the exact accented characters still matches too.

/** Fold a string for search: strip diacritics (NFKD + drop combining
 * marks) and lowercase. */
export function normalizeForSearch(s: string): string {
  return s.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();
}

/** Accent- and case-insensitive substring match. An empty or whitespace
 * query matches everything. */
export function searchMatches(haystack: string, query: string): boolean {
  return normalizeForSearch(haystack).includes(normalizeForSearch(query.trim()));
}
