/**
 * The opt-in second search layer for Preferences: the user's own saved text —
 * prompt bodies, the writer prompt library, and writer template names.
 *
 * Loaded on demand when the "search your saved text" option is enabled, so
 * plain settings search never does network fetches. Individual sources are
 * allowed to fail (e.g. no SDK configured yet) — failures are logged through
 * the backend bridge and the remaining sources still contribute.
 */

import { getPrompt, listWriterLibraryPrompts, listWriterTemplates } from "./tauri";
import { logFrontendEvent } from "./logBridge";
import type { PaneId, PreferenceHit } from "./preferencesNav";
import { categoryOf, paneSpec, PREFERENCES_NAV } from "./preferencesNav";

export interface UserContentEntry {
  paneId: PaneId;
  /** Display label of the entry, e.g. `Saved prompt "Phase 1"`. */
  title: string;
  /** The searchable body. */
  text: string;
}

/** The four managed prompt editors and the panes their text belongs to. */
const PROMPT_PANES: { promptName: string; paneId: PaneId }[] = [
  { promptName: "system-prompt", paneId: "prompts.chat-system" },
  { promptName: "pdf-extraction", paneId: "prompts.pdf-extraction" },
  { promptName: "report-system", paneId: "writer.prompt" },
  { promptName: "report-full-draft", paneId: "writer.whole-report" },
];

export async function loadUserContent(): Promise<UserContentEntry[]> {
  const sources: Promise<UserContentEntry[]>[] = [
    ...PROMPT_PANES.map(async ({ promptName, paneId }) => {
      const text = await getPrompt(promptName);
      return [{ paneId, title: `${paneSpec(paneId).title} text`, text }];
    }),
    listWriterLibraryPrompts().then((prompts) =>
      prompts.map((prompt) => ({
        paneId: "writer.prompt-library" as PaneId,
        title: `Saved prompt "${prompt.name}"`,
        text: `${prompt.name}\n${prompt.body}`,
      }))
    ),
    listWriterTemplates().then((templates) =>
      templates.map((template) => ({
        paneId: "writer.templates" as PaneId,
        title: `Template "${template.name}"`,
        text: template.name,
      }))
    ),
  ];

  const settled = await Promise.allSettled(sources);
  const entries: UserContentEntry[] = [];
  for (const result of settled) {
    if (result.status === "fulfilled") {
      entries.push(...result.value);
    } else {
      logFrontendEvent(
        "warn",
        `preferences saved-text search source failed: ${result.reason}`
      );
    }
  }
  return entries;
}

export interface ContentHit extends PreferenceHit {
  /** Text surrounding the match, for the result row. */
  snippet: string;
}

/** Case-insensitive substring search over loaded saved text. */
export function searchUserContent(
  entries: UserContentEntry[],
  rawQuery: string
): ContentHit[] {
  const query = rawQuery.trim().toLowerCase();
  if (query === "") return [];

  const hits: ContentHit[] = [];
  for (const entry of entries) {
    const haystack = entry.text.toLowerCase();
    const index = haystack.indexOf(query);
    if (index < 0 && !entry.title.toLowerCase().includes(query)) continue;
    const categoryId = categoryOf(entry.paneId);
    const category = PREFERENCES_NAV.find((c) => c.id === categoryId);
    hits.push({
      kind: "field",
      categoryId,
      paneId: entry.paneId,
      anchor: null,
      title: entry.title,
      context: `${category?.title ?? ""} › ${paneSpec(entry.paneId).title}`,
      snippet: snippetAround(entry.text, Math.max(index, 0), query.length),
    });
  }
  return hits;
}

function snippetAround(text: string, index: number, length: number): string {
  const start = Math.max(0, index - 32);
  const end = Math.min(text.length, index + length + 48);
  const clipped = text.slice(start, end).replace(/\s+/g, " ").trim();
  return `${start > 0 ? "…" : ""}${clipped}${end < text.length ? "…" : ""}`;
}
