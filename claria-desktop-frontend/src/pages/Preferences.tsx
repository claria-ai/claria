import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type ReactNode,
} from "react";
import {
  getPrompt,
  getWriterTrustRules,
  savePrompt,
  deletePrompt,
  setPreferredModel,
  getLocalTranscriptionStatus,
  downloadLocalModel,
  deleteLocalModel,
  deleteLegacyTranscriptionModels,
  saveLocalTranscriptionSettings,
  loadConfig,
  setHourlyCostData,
  getCostAndUsage,
  savePreferencesPatch,
  fetchCloudPreferences,
  uploadWriterTemplate,
  renameWriterTemplate,
  deleteWriterTemplate,
  exportPreferences,
  importPreferences,
  saveWriterLibraryPrompt,
  deleteWriterLibraryPrompt,
  type ChatStreamMode,
  type ConfigInfo,
  type DraftPipelinePreferences,
  type PlanGateMode,
  type EffortPreference,
  type ModelTuningPreferences,
  type LocalBackend,
  type LocalKvPrecision,
  type LocalModelId,
  type LocalModelInfo,
  type LocalTranscriptionSettings,
  type LocalTranscriptionStatus,
  type ModelDownloadProgress,
  type ReportAuthoringPreferences,
  type TranscriptionLanguage,
  type TranscriptionPreferences,
  type WriterPrompt,
  type WriterTemplateView,
} from "../lib/tauri";
import { logFrontendEvent } from "../lib/logBridge";
import { useChatModels } from "../lib/chatModels";
import { costErrorMessage } from "../lib/costErrors";
import { useAsyncLoad } from "../lib/useAsyncLoad";
import { useSaveOnLeave } from "../lib/useSaveOnLeave";
import { useWriterPrompts } from "../lib/useWriterPrompts";
import { useWriterTemplates } from "../lib/useWriterTemplates";
import { formatDateTime, formatFileSize } from "../lib/format";
import { preferencesVersions, promptVersions } from "../lib/versions";
import {
  categoryOf,
  defaultOpenPanes,
  paneSpec,
  searchPreferencesNav,
  PREFERENCES_NAV,
  WRITER_FOCUS_PANES,
  type CategoryId,
  type CategorySpec,
  type PaneId,
  type PreferenceHit,
} from "../lib/preferencesNav";
import {
  loadUserContent,
  searchUserContent,
  type ContentHit,
} from "../lib/preferencesSearchContent";
import EditableName from "../components/EditableName";
import ModelSelect from "../components/ModelSelect";
import PreferencesSection from "../components/PreferencesSection";
import ProgressBar from "../components/ProgressBar";
import {
  BackButton,
  ComposeIcon,
  DollarIcon,
  MicrophoneIcon,
  PromptIcon,
  SearchIcon,
  SparkleIcon,
  TrashIcon,
} from "../components/icons";
import Spinner from "../components/Spinner";
import { ErrorBanner, LoadingCard } from "../components/StateCards";
import VersionHistoryModal from "../components/VersionHistoryModal";
import type { Page, PreferencesWriterSection } from "../App";

// ---------------------------------------------------------------------------
// Pane plumbing: category → pane components, open state, search reveal
// ---------------------------------------------------------------------------

const CATEGORY_ICONS: Record<
  CategorySpec["icon"],
  ComponentType<{ className?: string }>
> = {
  sparkle: SparkleIcon,
  prompt: PromptIcon,
  compose: ComposeIcon,
  microphone: MicrophoneIcon,
  dollar: DollarIcon,
};

/** Accordion open state, provided by the page so search can force-open. */
const PaneControlContext = createContext<{
  isOpen: (id: PaneId) => boolean;
  setOpen: (id: PaneId, open: boolean) => void;
}>({ isOpen: () => false, setOpen: () => {} });

/**
 * One pane of a category: the accordion card, with its title and blurb pulled
 * from `PREFERENCES_NAV` (the single source of truth search also reads), a
 * "This Mac" badge for machine-local panes, and the `data-pane` container
 * search hits scroll to.
 */
function NavPane({
  paneId,
  summary,
  testId,
  contentClassName,
  children,
}: {
  paneId: PaneId;
  /** Small annotation rendered beside the title (current value, status). */
  summary?: ReactNode;
  testId?: string;
  contentClassName?: string;
  children: ReactNode;
}) {
  const spec = paneSpec(paneId);
  const { isOpen, setOpen } = useContext(PaneControlContext);
  const badge = spec.machineLocal ? (
    <span className="rounded-full bg-gray-200 px-1.5 py-0.5 text-[10px] font-medium text-gray-600">
      This Mac
    </span>
  ) : null;
  return (
    <div data-pane={paneId}>
      <PreferencesSection
        title={spec.title}
        summary={
          badge || summary ? (
            <>
              {badge}
              {summary}
            </>
          ) : undefined
        }
        open={isOpen(paneId)}
        onToggle={(next) => setOpen(paneId, next)}
        testId={testId}
        contentClassName={contentClassName}
        className="border border-gray-200 rounded-lg bg-white"
      >
        {spec.blurb && <p className="text-xs text-gray-400 mb-3">{spec.blurb}</p>}
        {children}
      </PreferencesSection>
    </div>
  );
}

export default function Preferences({
  navigate,
  focusSection = null,
  backPage = "start",
}: {
  navigate: (page: Page) => void;
  focusSection?: PreferencesWriterSection | null;
  backPage?: Page;
}) {
  const focusPane = focusSection ? WRITER_FOCUS_PANES[focusSection] : null;

  const [activeCategory, setActiveCategory] = useState<CategoryId>(
    focusPane ? categoryOf(focusPane) : PREFERENCES_NAV[0].id
  );
  const [openPanes, setOpenPanes] = useState<ReadonlySet<PaneId>>(() => {
    const open = new Set<PaneId>(defaultOpenPanes());
    if (focusPane) open.add(focusPane);
    return open;
  });
  const setPaneOpen = useCallback((id: PaneId, open: boolean) => {
    setOpenPanes((prev) => {
      if (prev.has(id) === open) return prev;
      const next = new Set(prev);
      if (open) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);
  const paneControl = useMemo(
    () => ({ isOpen: (id: PaneId) => openPanes.has(id), setOpen: setPaneOpen }),
    [openPanes, setPaneOpen]
  );

  // Bumped after a preferences import or version restore; keys the category
  // subtree so every mounted section refetches its freshly changed values.
  const [reloadNonce, setReloadNonce] = useState(0);

  // Search over the static index, plus (opt-in) the user's saved text.
  const [query, setQuery] = useState("");
  const [includeSaved, setIncludeSaved] = useState(false);
  const savedContent = useAsyncLoad(includeSaved ? loadUserContent : null, [
    includeSaved,
  ]);
  const searching = query.trim() !== "";
  const staticHits = useMemo(() => searchPreferencesNav(query), [query]);
  const contentHits = useMemo(
    () =>
      includeSaved && savedContent.data
        ? searchUserContent(savedContent.data, query)
        : [],
    [includeSaved, savedContent.data, query]
  );

  // A search hit (or a Writing-tab jump) lands here: category selected, pane
  // opened, then scroll to and flash the matched control once it has painted.
  // A focus pane can only arrive at mount — Preferences unmounts on
  // navigation — so it seeds the initial reveal rather than an effect.
  const [pendingReveal, setPendingReveal] = useState<{
    paneId: PaneId;
    anchor: string | null;
  } | null>(focusPane ? { paneId: focusPane, anchor: null } : null);

  function openHit(hit: PreferenceHit) {
    setQuery("");
    setActiveCategory(hit.categoryId);
    if (hit.paneId) {
      setPaneOpen(hit.paneId, true);
      setPendingReveal({ paneId: hit.paneId, anchor: hit.anchor });
    }
  }

  useEffect(() => {
    if (!pendingReveal) return;
    let inner = 0;
    // Double rAF: the pane must be laid out before we can scroll to it.
    const outer = requestAnimationFrame(() => {
      inner = requestAnimationFrame(() => {
        const pane = document.querySelector(
          `[data-pane="${pendingReveal.paneId}"]`
        );
        const target =
          (pendingReveal.anchor
            ? pane?.querySelector(
                `[data-pref-anchor="${pendingReveal.anchor}"]`
              )
            : pane) ?? pane;
        if (target instanceof HTMLElement) {
          target.scrollIntoView?.({ behavior: "smooth", block: "center" });
          target.classList.remove("pref-flash");
          void target.offsetWidth;
          target.classList.add("pref-flash");
        }
        setPendingReveal(null);
      });
    });
    return () => {
      cancelAnimationFrame(outer);
      cancelAnimationFrame(inner);
    };
  }, [pendingReveal]);

  return (
    <div className="flex h-screen">
      {/* Sidebar: search + category list */}
      <aside className="flex w-64 shrink-0 flex-col border-r border-gray-200 bg-gray-100/80">
        <div className="flex items-center gap-3 p-4 pb-2">
          <BackButton onClick={() => navigate(backPage)} />
          <h2 className="text-lg font-bold">Preferences</h2>
        </div>

        <div className="px-3 pb-2">
          <div className="flex items-center gap-2 rounded-lg border border-gray-300 bg-white px-2.5 py-1.5">
            <SearchIcon className="h-3.5 w-3.5 shrink-0 text-gray-400" />
            <input
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") setQuery("");
              }}
              placeholder="Search settings"
              aria-label="Search settings"
              className="w-full bg-transparent text-sm focus:outline-none"
            />
          </div>
          <label className="mt-1.5 flex items-start gap-1.5 px-1 text-xs text-gray-500">
            <input
              type="checkbox"
              checked={includeSaved}
              onChange={(e) => setIncludeSaved(e.target.checked)}
              className="mt-0.5"
            />
            Also search your prompts &amp; saved text
          </label>
        </div>

        <nav className="flex-1 space-y-0.5 overflow-y-auto px-3 py-1">
          {PREFERENCES_NAV.map((category) => {
            const Icon = CATEGORY_ICONS[category.icon];
            const active = !searching && category.id === activeCategory;
            return (
              <button
                key={category.id}
                type="button"
                onClick={() => {
                  setQuery("");
                  setActiveCategory(category.id);
                }}
                className={`flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-sm transition-colors ${
                  active
                    ? "bg-blue-600 text-white"
                    : "text-gray-800 hover:bg-gray-200"
                }`}
              >
                <span
                  className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-white ${category.tint}`}
                >
                  <Icon className="h-4 w-4" />
                </span>
                {category.title}
              </button>
            );
          })}
        </nav>

        <PreferencesFileTools
          onChanged={() => setReloadNonce((nonce) => nonce + 1)}
        />

        {/* Replaces the old cross-machine sync banner. */}
        <p className="border-t border-gray-200 p-3 text-xs text-gray-400">
          Settings are stored in your S3 bucket and follow you across
          computers. Panes marked &ldquo;This Mac&rdquo; stay on this computer.
        </p>
      </aside>

      {/* Content: search results, or the active category. Inactive categories
          stay mounted (hidden) so each section fetches once per visit and
          in-progress edits survive switching categories. */}
      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-2xl p-8">
          {searching && (
            <SearchResults
              query={query}
              hits={staticHits}
              contentHits={contentHits}
              savedEnabled={includeSaved}
              savedLoading={savedContent.loading}
              savedError={savedContent.error}
              onPick={openHit}
            />
          )}
          <PaneControlContext.Provider value={paneControl} key={reloadNonce}>
            {PREFERENCES_NAV.map((category) => (
              <div
                key={category.id}
                hidden={searching || category.id !== activeCategory}
              >
                <header className="mb-4">
                  <h3 className="text-xl font-semibold text-gray-900">
                    {category.title}
                  </h3>
                  <p className="text-sm text-gray-500">{category.blurb}</p>
                </header>
                <div className="space-y-4">
                  <CategoryPanes category={category} />
                </div>
              </div>
            ))}
          </PaneControlContext.Provider>
        </div>
      </main>
    </div>
  );
}

/**
 * Renders a category's panes. Consecutive panes served by one component
 * (e.g. the three on-device transcription panes sharing engine state) mount
 * that component once.
 */
function CategoryPanes({ category }: { category: CategorySpec }) {
  const rendered = new Set<ComponentType>();
  return (
    <>
      {category.panes.map((pane) => {
        const Section = PANE_COMPONENTS[pane.id];
        if (rendered.has(Section)) return null;
        rendered.add(Section);
        return <Section key={pane.id} />;
      })}
    </>
  );
}

function SearchResults({
  query,
  hits,
  contentHits,
  savedEnabled,
  savedLoading,
  savedError,
  onPick,
}: {
  query: string;
  hits: PreferenceHit[];
  contentHits: ContentHit[];
  savedEnabled: boolean;
  savedLoading: boolean;
  savedError: string | null;
  onPick: (hit: PreferenceHit) => void;
}) {
  const all: (PreferenceHit | ContentHit)[] = [...hits, ...contentHits];
  return (
    <div>
      <header className="mb-4">
        <h3 className="text-xl font-semibold text-gray-900">Search</h3>
        <p className="text-sm text-gray-500">
          Results for &ldquo;{query.trim()}&rdquo;
        </p>
      </header>
      {all.length === 0 && !savedLoading ? (
        <p className="rounded-lg border border-dashed border-gray-300 bg-white p-6 text-center text-sm text-gray-500">
          No settings match &ldquo;{query.trim()}&rdquo;.
        </p>
      ) : (
        <div
          className="divide-y divide-gray-100 rounded-lg border border-gray-200 bg-white"
          data-testid="pref-search-results"
        >
          {all.map((hit, index) => (
            <button
              key={`${hit.paneId ?? hit.categoryId}:${hit.anchor ?? ""}:${index}`}
              type="button"
              onClick={() => onPick(hit)}
              className="flex w-full flex-col items-start gap-0.5 px-4 py-3 text-left hover:bg-blue-50"
            >
              <span className="text-sm font-medium text-gray-900">
                {hit.title}
              </span>
              <span className="text-xs text-gray-400">{hit.context}</span>
              {"snippet" in hit && (
                <span className="text-xs text-gray-500">{hit.snippet}</span>
              )}
            </button>
          ))}
        </div>
      )}
      {savedEnabled && savedLoading && (
        <p className="mt-3 flex items-center gap-1.5 text-xs text-gray-400">
          <Spinner /> Searching your saved text…
        </p>
      )}
      {savedEnabled && savedError && (
        <ErrorBanner
          message={`Could not search saved text: ${savedError}`}
          className="mt-3"
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Preferences file tools: export, import, and version history with diff for
// _state/preferences.json — a support artifact and a one-file backup.
// ---------------------------------------------------------------------------

function PreferencesFileTools({ onChanged }: { onChanged: () => void }) {
  const { setPreferredModelId } = useChatModels();
  const [busy, setBusy] = useState<"export" | "import" | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showVersions, setShowVersions] = useState(false);

  async function handleExport() {
    setBusy("export");
    setStatus(null);
    setError(null);
    try {
      const saved = await exportPreferences();
      if (saved) setStatus("Preferences exported.");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  async function handleImport() {
    if (
      !window.confirm(
        "Replace your synced preferences with a file? The current values stay in version history."
      )
    ) {
      return;
    }
    setBusy("import");
    setStatus(null);
    setError(null);
    try {
      const imported = await importPreferences();
      if (imported) {
        // The preferred model lives in app-wide context, not section state,
        // so a remount alone would keep showing the pre-import pick.
        setPreferredModelId(imported.preferred_model_id ?? null);
        setStatus("Preferences imported.");
        onChanged();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  async function handleRestored() {
    try {
      const info = await loadConfig();
      setPreferredModelId(info.preferred_model_id ?? null);
    } catch (e) {
      logFrontendEvent("warn", `config reload after preferences restore failed: ${e}`);
    }
    setStatus("Previous version restored.");
    onChanged();
  }

  const buttonClass =
    "px-2 py-1 text-xs text-gray-600 border border-gray-300 rounded-md bg-white hover:bg-gray-50 disabled:opacity-50";

  return (
    <div className="border-t border-gray-200 p-3 space-y-2">
      <p className="text-xs font-medium text-gray-500">Preferences file</p>
      <div className="flex items-center gap-1.5">
        <button
          type="button"
          onClick={() => void handleExport()}
          disabled={busy !== null}
          className={buttonClass}
        >
          {busy === "export" ? "Exporting…" : "Export…"}
        </button>
        <button
          type="button"
          onClick={() => void handleImport()}
          disabled={busy !== null}
          className={buttonClass}
        >
          {busy === "import" ? "Importing…" : "Import…"}
        </button>
        <button
          type="button"
          onClick={() => {
            setStatus(null);
            setError(null);
            setShowVersions(true);
          }}
          disabled={busy !== null}
          className={buttonClass}
        >
          History
        </button>
      </div>
      {status && <p className="text-xs text-green-600">{status}</p>}
      {error && <p className="text-xs text-red-600 break-words">{error}</p>}

      {showVersions && (
        <VersionHistoryModal
          title="Preferences File Versions"
          source={preferencesVersions()}
          enableCompare
          onClose={() => setShowVersions(false)}
          onRestored={handleRestored}
          onError={setError}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Preferred model
// ---------------------------------------------------------------------------

function PreferredModelSection() {
  const {
    models: chatModels,
    loading: chatModelsLoading,
    error: chatModelsError,
    preferredModelId,
    setPreferredModelId,
  } = useChatModels();

  const [modelSaving, setModelSaving] = useState(false);
  const [modelError, setModelError] = useState<string | null>(null);

  async function handleModelChange(modelId: string) {
    const value = modelId || null;
    setModelSaving(true);
    setModelError(null);
    try {
      await setPreferredModel(value);
      setPreferredModelId(value);
    } catch (e) {
      setModelError(String(e));
    } finally {
      setModelSaving(false);
    }
  }

  return (
    <NavPane
      paneId="claude.preferred-model"
      summary={
        preferredModelId && chatModels.length > 0 ? (
          <span className="text-xs text-gray-400">
            {chatModels.find((m) => m.model_id === preferredModelId)?.name ??
              preferredModelId}
          </span>
        ) : undefined
      }
    >
      {chatModelsLoading ? (
        <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
          <Spinner />
          <span>Loading models...</span>
        </div>
      ) : chatModelsError ? (
        <ErrorBanner message={chatModelsError} className="" />
      ) : (
        <>
          <div data-pref-anchor="preferred_model">
            <select
              value={preferredModelId ?? ""}
              onChange={(e) => handleModelChange(e.target.value)}
              disabled={modelSaving}
              className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:bg-gray-50"
            >
              <option value="">Use first available model</option>
              {chatModels.map((m) => (
                <option key={m.model_id} value={m.model_id}>
                  {m.name}
                </option>
              ))}
            </select>
          </div>
          {modelError && <ErrorBanner message={modelError} className="mt-3" />}
        </>
      )}
    </NavPane>
  );
}

/**
 * Save-state footer for a section that saves when the user leaves it
 * (`useSaveOnLeave`), so edits read as intentional drafts, not lost clicks.
 */
function SectionSaveStatus({
  syncing,
  dirty,
  syncedOnce,
}: {
  syncing: boolean;
  dirty: boolean;
  syncedOnce: boolean;
}) {
  if (!syncing && !dirty && !syncedOnce) return null;
  return (
    <p className="text-xs text-gray-400 pt-2 border-t border-gray-100 flex items-center gap-1.5">
      {syncing ? (
        <>
          <Spinner /> Saving...
        </>
      ) : dirty ? (
        "Unsaved changes — saved when you leave this section."
      ) : (
        "Saved. Other computers pick this up on restart."
      )}
    </p>
  );
}

// ---------------------------------------------------------------------------
// Chat streaming — how much of a reply appears at a time
// ---------------------------------------------------------------------------

const CHAT_STREAM_OPTIONS: {
  value: ChatStreamMode;
  label: string;
  description: string;
}[] = [
  {
    value: "paragraph",
    label: "A paragraph at a time",
    description:
      "The reply appears in finished blocks. Formatting, lists, and tables are never caught half-written.",
  },
  {
    value: "token",
    label: "Word by word",
    description:
      "The reply appears as fast as the model produces it. Nothing arrives sooner overall — it just arrives in smaller pieces.",
  },
  {
    value: "off",
    label: "All at once",
    description:
      "Nothing appears until the reply is finished. Longest wait, no movement on screen.",
  },
];

function ChatStreamingSection() {
  const [snapshot, setSnapshot] = useState<ChatStreamMode | null>(null);
  const [draft, setDraft] = useState<ChatStreamMode | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [syncedOnce, setSyncedOnce] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const info = await fetchCloudPreferences();
      setSnapshot(info.chat_streaming);
      setDraft(info.chat_streaming);
    } catch (e) {
      try {
        const info = await loadConfig();
        setSnapshot(info.chat_streaming);
        setDraft(info.chat_streaming);
      } catch (fallbackError) {
        setLoadError(String(fallbackError ?? e));
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const dirty = snapshot != null && draft != null && snapshot !== draft;

  const sync = useCallback(async (next: ChatStreamMode) => {
    setSyncing(true);
    setSaveError(null);
    try {
      const updated = await savePreferencesPatch({ chat_streaming: next });
      setSnapshot(updated.chat_streaming);
      setSyncedOnce(true);
    } catch (e) {
      // Also logged through the bridge: a flush finishing after unmount has
      // no UI left to show the banner in.
      logFrontendEvent("error", `chat streaming save failed: ${e}`);
      setSaveError(String(e));
    } finally {
      setSyncing(false);
    }
  }, []);

  // The draft that still needs saving; null when clean or already flushed.
  const pendingRef = useRef<ChatStreamMode | null>(null);
  useEffect(() => {
    pendingRef.current = dirty && draft ? draft : null;
  }, [dirty, draft]);
  const flush = useCallback(() => {
    const pending = pendingRef.current;
    if (!pending) return;
    pendingRef.current = null;
    void sync(pending);
  }, [sync]);
  const { containerRef, onContainerBlur } = useSaveOnLeave(flush);

  const active = CHAT_STREAM_OPTIONS.find((option) => option.value === draft);

  return (
    <NavPane
      paneId="claude.chat-streaming"
      summary={
        active ? (
          <span className="text-xs text-gray-400">{active.label}</span>
        ) : undefined
      }
      contentClassName="border-t border-gray-100 p-4 space-y-3"
    >
      {loading ? (
        <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
          <Spinner />
          <span>Loading chat streaming...</span>
        </div>
      ) : draft == null ? (
        <ErrorBanner
          message={loadError ?? "Could not load the chat streaming setting."}
          className=""
        />
      ) : (
        <div
          ref={containerRef}
          onBlur={onContainerBlur}
          className="space-y-3"
        >
          <div className="space-y-3" data-pref-anchor="chat_streaming">
            {CHAT_STREAM_OPTIONS.map((option) => (
              <label key={option.value} className="flex items-start gap-2">
                <input
                  type="radio"
                  name="chat-streaming"
                  value={option.value}
                  checked={draft === option.value}
                  onChange={() => setDraft(option.value)}
                  className="mt-0.5"
                />
                <span>
                  <span className="text-sm text-gray-700 block">
                    {option.label}
                  </span>
                  <span className="text-xs text-gray-500 block">
                    {option.description}
                  </span>
                </span>
              </label>
            ))}
          </div>
          {saveError ? (
            <ErrorBanner
              message={`Could not save chat streaming: ${saveError}`}
              onRetry={() => {
                if (draft) void sync(draft);
              }}
              className=""
            />
          ) : (
            <SectionSaveStatus
              syncing={syncing}
              dirty={dirty}
              syncedOnce={syncedOnce}
            />
          )}
        </div>
      )}
    </NavPane>
  );
}

// ---------------------------------------------------------------------------
// Model tuning — opt-in adaptive reasoning, effort, and temperature
// ---------------------------------------------------------------------------

const EFFORT_OPTIONS: { value: "" | EffortPreference; label: string }[] = [
  { value: "", label: "Model default (high)" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "max", label: "Max" },
];

function ModelTuningSection() {
  const [snapshot, setSnapshot] = useState<ConfigInfo | null>(null);
  const [draft, setDraft] = useState<ModelTuningPreferences | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const info = await fetchCloudPreferences();
      setSnapshot(info);
      setDraft(info.model_tuning);
    } catch (e) {
      try {
        const info = await loadConfig();
        setSnapshot(info);
        setDraft(info.model_tuning);
      } catch (fallbackError) {
        setLoadError(String(fallbackError ?? e));
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const dirty =
    snapshot != null &&
    draft != null &&
    JSON.stringify(snapshot.model_tuning) !== JSON.stringify(draft);
  const temperatureInvalid =
    draft?.temperature != null &&
    (draft.temperature < 0 || draft.temperature > 1);

  async function save() {
    if (!snapshot || !draft || temperatureInvalid) return;
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      // Patch-save: only the model tuning section travels.
      const updated = await savePreferencesPatch({ model_tuning: draft });
      setSnapshot(updated);
      setDraft(updated.model_tuning);
      setSaved(true);
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <NavPane
      paneId="claude.model-tuning"
      summary={
        draft && (draft.reasoning_enabled || draft.effort || draft.temperature != null) ? (
          <span className="text-xs text-gray-400">
            {[
              draft.reasoning_enabled ? "Adaptive reasoning on" : null,
              draft.effort ? `effort: ${draft.effort}` : null,
              draft.temperature != null ? `temp: ${draft.temperature}` : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </span>
        ) : undefined
      }
      contentClassName="border-t border-gray-100 p-4 space-y-4"
    >
      {loading ? (
        <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
          <Spinner />
          <span>Loading model tuning...</span>
        </div>
      ) : !draft ? (
        <ErrorBanner
          message={loadError ?? "Could not load model tuning preferences."}
          className=""
        />
      ) : (
        <>
          <label
            className="flex items-start gap-2"
            data-pref-anchor="reasoning_enabled"
          >
            <input
              type="checkbox"
              checked={draft.reasoning_enabled}
              onChange={(e) => {
                setSaved(false);
                setDraft({ ...draft, reasoning_enabled: e.target.checked });
              }}
              className="mt-0.5"
            />
            <span>
              <span className="text-sm text-gray-700 block">
                Adaptive reasoning
              </span>
              <span className="text-xs text-gray-500 block">
                Lets supported models (Claude 4.6 and newer) think before
                answering, which can improve report quality. Reasoning tokens
                bill as output and count against the response budget, so
                turns cost more and take longer.
              </span>
            </span>
          </label>

          <div data-pref-anchor="effort">
            <label className="text-sm text-gray-700 block mb-1">Effort</label>
            <select
              value={draft.effort ?? ""}
              onChange={(e) => {
                setSaved(false);
                setDraft({
                  ...draft,
                  effort: (e.target.value || null) as EffortPreference | null,
                });
              }}
              className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            >
              {EFFORT_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
            <p className="text-xs text-gray-500 mt-1">
              How much reasoning supported models put into each response.
              Sent to Opus 4.5 and every model from Claude 4.6 on.
            </p>
          </div>

          <div data-pref-anchor="temperature">
            <label className="text-sm text-gray-700 block mb-1">
              Temperature
            </label>
            <input
              type="number"
              min={0}
              max={1}
              step={0.1}
              placeholder="Model default"
              value={draft.temperature ?? ""}
              onChange={(e) => {
                setSaved(false);
                setDraft({
                  ...draft,
                  temperature:
                    e.target.value === "" ? null : Number(e.target.value),
                });
              }}
              className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            />
            <p className="text-xs text-gray-500 mt-1">
              0.0 to 1.0; leave blank for the model default. Only sent to
              model generations that accept it (through Claude 4.6) — newer
              models always use their own default.
            </p>
          </div>

          {temperatureInvalid && (
            <ErrorBanner
              message="Temperature must be between 0.0 and 1.0."
              className=""
            />
          )}
          {saveError && <ErrorBanner message={saveError} className="" />}

          <div className="flex items-center justify-end gap-3">
            {saved && !dirty && (
              <span className="text-xs text-green-600">Saved</span>
            )}
            <button
              onClick={save}
              disabled={!dirty || saving || temperatureInvalid}
              className="px-4 py-1.5 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
            >
              {saving ? "Saving..." : "Save"}
            </button>
          </div>
        </>
      )}
    </NavPane>
  );
}

// ---------------------------------------------------------------------------
// Reusable prompt editor accordion
// ---------------------------------------------------------------------------

function PromptEditor({
  paneId,
  promptName,
  fixedRules,
}: {
  paneId: PaneId;
  promptName: string;
  /** Read-only rules Claria always appends after the editable body. */
  fixedRules?: string;
}) {
  const spec = paneSpec(paneId);
  const [content, setContent] = useState("");
  const [saving, setSaving] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);

  const [showVersions, setShowVersions] = useState(false);

  const {
    data: loadedContent,
    loading,
    error: loadError,
    reload,
  } = useAsyncLoad(() => getPrompt(promptName), [promptName]);
  const error = loadError ?? actionError;

  // Adopt each freshly loaded prompt as the editable copy.
  useEffect(() => {
    if (loadedContent != null) {
      setContent(loadedContent);
      setDirty(false);
    }
  }, [loadedContent]);

  async function handleSave() {
    setSaving(true);
    setActionError(null);
    try {
      await savePrompt(promptName, content);
      setDirty(false);
    } catch (e) {
      setActionError(String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleReset() {
    setSaving(true);
    setActionError(null);
    try {
      await deletePrompt(promptName);
      const text = await getPrompt(promptName);
      setContent(text);
      setDirty(false);
    } catch (e) {
      setActionError(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <>
      <NavPane paneId={paneId}>
        {loading ? (
          <LoadingCard>Loading prompt...</LoadingCard>
        ) : (
          <>
            <textarea
              value={content}
              onChange={(e) => {
                setContent(e.target.value);
                setDirty(true);
              }}
              disabled={saving}
              className="w-full min-h-[216px] px-3 py-2 text-sm font-mono border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent resize-y disabled:bg-gray-50"
            />

            {fixedRules && (
              <div className="mt-3">
                <p className="text-xs text-gray-400 mb-1">
                  Claria always appends these trust rules after your prompt.
                  They are not editable and cannot be removed:
                </p>
                {/* 184px = 8px top padding + 11 exact 16px text-xs line
                    boxes, so the scroll fold never slices a line. */}
                <pre className="w-full max-h-[184px] overflow-auto px-3 py-2 text-xs font-mono text-gray-500 bg-gray-50 border border-gray-200 rounded-lg whitespace-pre-wrap">
                  {fixedRules}
                </pre>
              </div>
            )}

            {error && (
              <ErrorBanner message={error} className="mt-3" />
            )}

            <div className="flex justify-between mt-3">
              <div className="flex gap-2">
                <button
                  onClick={handleReset}
                  disabled={loading || saving}
                  className="px-3 py-1.5 text-sm text-amber-600 border border-amber-300 rounded-lg hover:bg-amber-50 transition-colors disabled:opacity-50"
                >
                  {saving ? "Resetting..." : "Reset to Default"}
                </button>
                <button
                  onClick={() => setShowVersions(true)}
                  disabled={saving}
                  className="px-3 py-1.5 text-sm text-gray-600 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors disabled:opacity-50"
                >
                  Version History
                </button>
              </div>
              <button
                onClick={handleSave}
                disabled={loading || saving || !dirty}
                className="px-4 py-1.5 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
              >
                {saving ? "Saving..." : "Save"}
              </button>
            </div>
          </>
        )}
      </NavPane>

      {showVersions && (
        <VersionHistoryModal
          title={`${spec.title} Versions`}
          source={promptVersions(promptName)}
          onClose={() => setShowVersions(false)}
          onRestored={reload}
          onError={setActionError}
        />
      )}
    </>
  );
}

function ChatSystemPromptSection() {
  return <PromptEditor paneId="prompts.chat-system" promptName="system-prompt" />;
}

function PdfExtractionPromptSection() {
  return (
    <PromptEditor paneId="prompts.pdf-extraction" promptName="pdf-extraction" />
  );
}

/**
 * The two writer prompt editors, sharing one fetch of the fixed trust rules
 * that are displayed read-only under each so nothing about how the writer
 * runs is hidden.
 */
function WriterPromptEditors() {
  const { data: trustRules } = useAsyncLoad(() => getWriterTrustRules(), []);
  return (
    <>
      <PromptEditor
        paneId="writer.prompt"
        promptName="report-system"
        fixedRules={trustRules?.targeted}
      />
      <PromptEditor
        paneId="writer.whole-report"
        promptName="report-full-draft"
        fixedRules={trustRules?.full_draft}
      />
    </>
  );
}

// ---------------------------------------------------------------------------
// transcribe.cpp model management and machine-local inference settings
// ---------------------------------------------------------------------------

/**
 * The three on-device transcription panes (models, compute, decoding) share
 * one engine status fetch and one settings draft, so they render from a
 * single component mounted once per category.
 */
function LocalTranscriptionPanes() {
  const [status, setStatus] = useState<LocalTranscriptionStatus | null>(null);
  const [draft, setDraft] = useState<LocalTranscriptionSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [busyModel, setBusyModel] = useState<LocalModelId | null>(null);
  const [removingLegacy, setRemovingLegacy] = useState(false);
  const [progress, setProgress] = useState<ModelDownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const applyStatus = useCallback((next: LocalTranscriptionStatus) => {
    setStatus(next);
    setDraft(next.settings);
  }, []);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      applyStatus(await getLocalTranscriptionStatus());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [applyStatus]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function persist(next: LocalTranscriptionSettings) {
    setSaving(true);
    setError(null);
    try {
      applyStatus(await saveLocalTranscriptionSettings(next));
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleDownload(modelId: LocalModelId) {
    setBusyModel(modelId);
    setProgress(null);
    setError(null);
    try {
      applyStatus(
        await downloadLocalModel(modelId, (next) => setProgress(next)),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyModel(null);
      setProgress(null);
    }
  }

  async function handleDelete(modelId: LocalModelId) {
    setBusyModel(modelId);
    setError(null);
    try {
      applyStatus(await deleteLocalModel(modelId));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyModel(null);
    }
  }

  async function handleLegacyDelete() {
    setRemovingLegacy(true);
    setError(null);
    try {
      applyStatus(await deleteLegacyTranscriptionModels());
    } catch (e) {
      setError(String(e));
    } finally {
      setRemovingLegacy(false);
    }
  }

  function activate(model: LocalModelInfo) {
    if (!draft) return;
    void persist({ ...draft, speech_model: model.id });
  }

  const dirty =
    status != null &&
    draft != null &&
    JSON.stringify(status.settings) !== JSON.stringify(draft);
  const ready =
    status?.models.some((model) => model.active && model.downloaded) ?? false;
  const selectedBackendKind =
    draft?.backend === "cpu_accel" ? "accel" : draft?.backend;
  const selectedDevice =
    status && draft
      ? draft.gpu_device > 0
        ? status.devices.find((device) => device.index === draft.gpu_device)
        : status.devices.find((device) =>
            selectedBackendKind === "auto"
              ? !["cpu", "accel"].includes(device.kind)
              : device.kind === selectedBackendKind,
          )
      : undefined;
  const maxDeviceIndex = Math.max(
    0,
    ...(status?.devices.map((device) => device.index ?? 0) ?? []),
  );

  const loadingRow = (
    <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
      <Spinner /> <span>Checking local runtime and models...</span>
    </div>
  );

  // The engine settings draft spans the compute and decoding panes, so both
  // carry the same save button acting on the shared draft.
  const saveButton = (
    <div className="flex justify-end">
      <button
        onClick={() => draft && void persist(draft)}
        disabled={!dirty || saving || busyModel !== null}
        className="px-3 py-1.5 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50"
      >
        {saving ? "Saving..." : "Save local engine settings"}
      </button>
    </div>
  );

  return (
    <>
      <NavPane
        paneId="transcription.local-models"
        summary={
          ready ? <span className="text-xs text-green-600">Ready</span> : undefined
        }
        contentClassName="border-t border-gray-100 p-4 space-y-5"
      >
        {loading ? (
          loadingRow
        ) : status && draft ? (
          <>
            <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 text-xs text-gray-600 space-y-1">
              <p>
                transcribe.cpp {status.runtime_version} · {status.accelerated ? "GPU accelerated" : "CPU"}
              </p>
              {selectedDevice && (
                <p>
                  {selectedDevice.description || selectedDevice.name}
                  {selectedDevice.memory_total > 0
                    ? ` · ${formatFileSize(selectedDevice.memory_total)} memory`
                    : ""}
                </p>
              )}
            </div>

            <div data-pref-anchor="speech_model">
              <ModelGroup
                title="Memo speech model"
                description="Used only for on-device Record Memo transcription."
                models={status.models}
                busyModel={busyModel}
                progress={progress}
                disabled={saving || removingLegacy || dirty}
                onActivate={activate}
                onDownload={handleDownload}
                onDelete={handleDelete}
              />
            </div>

            {status.legacy_model_bytes > 0 && (
              <div
                className="border border-amber-200 bg-amber-50 rounded-lg p-3 flex items-start justify-between gap-3"
                data-pref-anchor="legacy_models"
              >
                <div>
                  <p className="text-sm text-amber-900">Legacy Candle model files</p>
                  <p className="text-xs text-amber-700 mt-0.5">
                    {formatFileSize(status.legacy_model_bytes)} from the previous
                    safetensors engine is no longer used.
                  </p>
                </div>
                <button
                  onClick={() => void handleLegacyDelete()}
                  disabled={removingLegacy || busyModel !== null || dirty}
                  className="px-2.5 py-1 text-xs text-red-700 border border-red-300 rounded-lg hover:bg-red-50 disabled:opacity-50"
                >
                  {removingLegacy ? "Removing..." : "Remove legacy files"}
                </button>
              </div>
            )}
          </>
        ) : null}

        {error && (
          <ErrorBanner
            message={error}
            onRetry={() => void refresh()}
            className=""
          />
        )}
      </NavPane>

      <NavPane
        paneId="transcription.local-compute"
        contentClassName="border-t border-gray-100 p-4 space-y-3"
      >
        {loading ? (
          loadingRow
        ) : status && draft ? (
          <>
            <div className="grid grid-cols-2 gap-3">
              <label className="text-xs text-gray-600" data-pref-anchor="backend">
                Backend
                <select
                  value={draft.backend}
                  onChange={(event) =>
                    setDraft({ ...draft, backend: event.target.value as LocalBackend })
                  }
                  className="mt-1 w-full px-2.5 py-2 text-sm border border-gray-300 rounded-lg bg-white"
                >
                  {status.backends.map((backend) => (
                    <option
                      key={backend.backend}
                      value={backend.backend}
                      disabled={!backend.available}
                    >
                      {backend.label}{backend.available ? "" : " (unavailable)"}
                    </option>
                  ))}
                </select>
              </label>
              <label className="text-xs text-gray-600" data-pref-anchor="gpu_device">
                Compute device index (0 = automatic)
                <input
                  type="number"
                  min={0}
                  max={maxDeviceIndex}
                  value={draft.gpu_device}
                  onChange={(event) =>
                    setDraft({ ...draft, gpu_device: Number(event.target.value) })
                  }
                  className="mt-1 w-full px-2.5 py-2 text-sm border border-gray-300 rounded-lg"
                />
              </label>
              <label className="text-xs text-gray-600" data-pref-anchor="cpu_threads">
                CPU threads (0 = automatic)
                <input
                  type="number"
                  min={0}
                  max={256}
                  value={draft.cpu_threads}
                  onChange={(event) =>
                    setDraft({ ...draft, cpu_threads: Number(event.target.value) })
                  }
                  className="mt-1 w-full px-2.5 py-2 text-sm border border-gray-300 rounded-lg"
                />
              </label>
              <label className="text-xs text-gray-600" data-pref-anchor="kv_precision">
                K/V cache precision
                <select
                  value={draft.kv_precision}
                  onChange={(event) =>
                    setDraft({
                      ...draft,
                      kv_precision: event.target.value as LocalKvPrecision,
                    })
                  }
                  className="mt-1 w-full px-2.5 py-2 text-sm border border-gray-300 rounded-lg bg-white"
                >
                  <option value="auto">Automatic</option>
                  <option value="f16">F16 (lower memory)</option>
                  <option value="f32">F32 (higher precision)</option>
                </select>
              </label>
              {status.devices.length > 0 && (
                <p className="col-span-2 text-xs text-gray-400">
                  Runtime devices: {status.devices.map((device) =>
                    `${device.index ?? "?"}: ${device.description || device.name}`,
                  ).join(" · ")}
                </p>
              )}
            </div>
            {saveButton}
          </>
        ) : null}
      </NavPane>

      <NavPane
        paneId="transcription.local-decoding"
        contentClassName="border-t border-gray-100 p-4 space-y-3"
      >
        {loading ? (
          loadingRow
        ) : status && draft ? (
          <>
            <label
              className="block text-xs text-gray-600"
              data-pref-anchor="initial_prompt"
            >
              Initial prompt / vocabulary hint
              <textarea
                value={draft.initial_prompt}
                onChange={(event) =>
                  setDraft({ ...draft, initial_prompt: event.target.value })
                }
                placeholder="Optional clinical terms, names, or context"
                className="mt-1 w-full min-h-20 px-2.5 py-2 text-sm border border-gray-300 rounded-lg"
              />
            </label>
            <label
              className="flex items-start gap-2 text-sm text-gray-700"
              data-pref-anchor="condition_on_previous_text"
            >
              <input
                type="checkbox"
                checked={draft.condition_on_previous_text}
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    condition_on_previous_text: event.target.checked,
                  })
                }
                className="mt-0.5"
              />
              Carry accepted text into each following 30-second window
            </label>
            <div className="grid grid-cols-2 gap-3">
              <NumberSetting
                label="Previous-context tokens"
                anchor="max_previous_context_tokens"
                value={draft.max_previous_context_tokens}
                min={0}
                max={448}
                step={1}
                onChange={(value) =>
                  setDraft({ ...draft, max_previous_context_tokens: value })
                }
              />
              <NumberSetting
                label="Temperature"
                anchor="temperature"
                value={draft.temperature}
                min={0}
                max={1}
                step={0.1}
                onChange={(value) => setDraft({ ...draft, temperature: value })}
              />
              <NumberSetting
                label="Temperature increment"
                anchor="temperature_increment"
                value={draft.temperature_increment}
                min={0}
                max={1}
                step={0.1}
                onChange={(value) =>
                  setDraft({ ...draft, temperature_increment: value })
                }
              />
              <NumberSetting
                label="Compression-ratio threshold"
                anchor="compression_ratio_threshold"
                value={draft.compression_ratio_threshold}
                min={0.1}
                max={100}
                step={0.1}
                onChange={(value) =>
                  setDraft({ ...draft, compression_ratio_threshold: value })
                }
              />
              <NumberSetting
                label="Log-probability threshold"
                anchor="log_probability_threshold"
                value={draft.log_probability_threshold}
                min={-100}
                max={0}
                step={0.1}
                onChange={(value) =>
                  setDraft({ ...draft, log_probability_threshold: value })
                }
              />
              <NumberSetting
                label="No-speech threshold"
                anchor="no_speech_threshold"
                value={draft.no_speech_threshold}
                min={0}
                max={1}
                step={0.05}
                onChange={(value) =>
                  setDraft({ ...draft, no_speech_threshold: value })
                }
              />
              <NumberSetting
                label="Sampling seed (0 = random)"
                anchor="seed"
                value={draft.seed}
                min={0}
                max={4294967295}
                step={1}
                onChange={(value) => setDraft({ ...draft, seed: value })}
              />
            </div>
            {saveButton}
          </>
        ) : null}
      </NavPane>
    </>
  );
}

function ModelGroup({
  title,
  description,
  models,
  busyModel,
  progress,
  disabled,
  onActivate,
  onDownload,
  onDelete,
}: {
  title: string;
  description: string;
  models: LocalModelInfo[];
  busyModel: LocalModelId | null;
  progress: ModelDownloadProgress | null;
  disabled: boolean;
  onActivate: (model: LocalModelInfo) => void;
  onDownload: (modelId: LocalModelId) => void;
  onDelete: (modelId: LocalModelId) => void;
}) {
  return (
    <div>
      <h4 className="text-sm font-medium text-gray-700">{title}</h4>
      <p className="text-xs text-gray-500 mt-0.5 mb-2">{description}</p>
      <div className="space-y-2">
        {models.map((model) => {
          const modelProgress = progress?.model_id === model.id ? progress : null;
          const percent = modelProgress
            ? Math.min(
                100,
                Math.round(
                  (modelProgress.downloaded_bytes / modelProgress.total_bytes) * 100,
                ),
              )
            : 0;
          return (
            <div
              key={model.id}
              className={`border rounded-lg p-3 ${
                model.active ? "border-green-300 bg-green-50/40" : "border-gray-200"
              }`}
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium text-gray-900">{model.label}</span>
                    {model.active && (
                      <span className="px-1.5 py-0.5 text-xs bg-green-100 text-green-700 rounded">
                        Active
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-gray-500 mt-0.5">{model.description}</p>
                  <p className="text-xs text-gray-400 mt-1">
                    {model.quantization} · {formatFileSize(model.download_size_bytes)} · {model.languages.join(", ")}
                  </p>
                  {model.model_path && (
                    <p className="text-xs text-gray-400 mt-0.5 break-all">{model.model_path}</p>
                  )}
                </div>
                <div className="flex gap-2 shrink-0">
                  {model.downloaded ? (
                    <>
                      {!model.active && (
                        <button
                          onClick={() => onActivate(model)}
                          disabled={disabled || busyModel !== null}
                          className="px-2.5 py-1 text-xs text-blue-600 border border-blue-300 rounded-lg disabled:opacity-50"
                        >
                          Activate
                        </button>
                      )}
                      <button
                        onClick={() => onDelete(model.id)}
                        disabled={disabled || busyModel !== null}
                        className="px-2.5 py-1 text-xs text-red-600 border border-red-300 rounded-lg disabled:opacity-50"
                      >
                        {busyModel === model.id ? "Removing..." : "Remove"}
                      </button>
                    </>
                  ) : (
                    <button
                      onClick={() => onDownload(model.id)}
                      disabled={disabled || busyModel !== null}
                      className="px-2.5 py-1 text-xs text-white bg-blue-600 rounded-lg disabled:opacity-50"
                    >
                      {busyModel === model.id ? `Downloading ${percent}%` : "Download"}
                    </button>
                  )}
                </div>
              </div>
              {modelProgress && (
                <ProgressBar
                  className="mt-2"
                  value={modelProgress.downloaded_bytes}
                  max={modelProgress.total_bytes}
                  label={`Downloading ${model.label}`}
                  valueText={`${percent}%`}
                  showValueText={false}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function NumberSetting({
  label,
  anchor,
  value,
  min,
  max,
  step,
  onChange,
}: {
  label: string;
  /** `data-pref-anchor` for search reveal. */
  anchor?: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="text-xs text-gray-600" data-pref-anchor={anchor}>
      {label}
      <input
        type="number"
        value={value}
        min={min}
        max={max}
        step={step}
        onChange={(event) => onChange(Number(event.target.value))}
        className="mt-1 w-full px-2.5 py-2 text-sm border border-gray-300 rounded-lg"
      />
    </label>
  );
}

// ---------------------------------------------------------------------------
// Cost Explorer settings
// ---------------------------------------------------------------------------

function CostExplorerSection() {
  const [hourlyEnabled, setHourlyEnabled] = useState<boolean | null>(null);
  const [loading, setLoading] = useState(true);
  const [verifying, setVerifying] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    loadConfig()
      .then((info) => setHourlyEnabled(info.hourly_cost_data))
      .catch(() => setHourlyEnabled(false))
      .finally(() => setLoading(false));
  }, []);

  async function handleToggle() {
    if (hourlyEnabled) {
      // Turning off — no verification needed
      setError(null);
      try {
        await setHourlyCostData(false);
        setHourlyEnabled(false);
      } catch (e) {
        setError(String(e));
      }
      return;
    }

    // Turning on — verify with a test hourly request
    setVerifying(true);
    setError(null);
    try {
      const yesterday = new Date();
      yesterday.setDate(yesterday.getDate() - 1);
      const today = new Date();
      const fmt = (d: Date) => {
        const y = d.getFullYear();
        const m = String(d.getMonth() + 1).padStart(2, "0");
        const day = String(d.getDate()).padStart(2, "0");
        return `${y}-${m}-${day}`;
      };
      await getCostAndUsage(fmt(yesterday), fmt(today), "hourly", false);
      await setHourlyCostData(true);
      setHourlyEnabled(true);
    } catch (e) {
      setError(
        costErrorMessage(e, {
          accessDenied:
            "Hourly data is not enabled for this account. In the AWS Console, go to " +
            'Billing → Cost Explorer → Settings and enable "Hourly and Resource Level Data".',
          dataUnavailable:
            "Hourly cost data is not available yet. Enable it in the AWS Console under " +
            "Billing → Cost Explorer → Settings, then wait up to 24 hours for data to appear.",
        })
      );
    } finally {
      setVerifying(false);
    }
  }

  return (
    <NavPane
      paneId="billing.cost-explorer"
      summary={
        hourlyEnabled ? (
          <span className="text-xs text-gray-400">Hourly enabled</span>
        ) : undefined
      }
    >
      {loading ? (
        <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
          <Spinner />
          <span>Loading...</span>
        </div>
      ) : (
        <label className="flex items-start gap-3" data-pref-anchor="hourly_cost_data">
          <input
            type="checkbox"
            checked={hourlyEnabled ?? false}
            onChange={handleToggle}
            disabled={verifying}
            className="mt-0.5 rounded border-gray-300"
          />
          <div className="flex-1">
            <span className="text-sm text-gray-900">
              Hourly data resolution
              {verifying && (
                <span className="ml-2 text-xs text-gray-400 inline-flex items-center gap-1">
                  <Spinner /> Verifying...
                </span>
              )}
            </span>
            <p className="text-xs text-gray-400 mt-0.5">
              Shows hourly cost breakdowns for the last 14 days. Must be enabled in
              AWS Console under Billing &rarr; Cost Explorer &rarr; Settings first.
            </p>
          </div>
        </label>
      )}

      {error && (
        <ErrorBanner message={error} className="mt-3" />
      )}
    </NavPane>
  );
}

// ---------------------------------------------------------------------------
// Writer templates: managed redacted DOCX presets stored in S3
// ---------------------------------------------------------------------------

function WriterTemplatesSection() {
  const {
    templates,
    setTemplates,
    loading,
    error: loadError,
    reload,
  } = useWriterTemplates();
  const [busy, setBusy] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const error = loadError ?? actionError;

  async function upload() {
    setBusy("upload");
    setActionError(null);
    try {
      const uploaded = await uploadWriterTemplate();
      if (uploaded) {
        setTemplates((current) => [uploaded, ...current]);
      }
    } catch (reason) {
      setActionError(String(reason));
    } finally {
      setBusy(null);
    }
  }

  async function rename(templateId: string, name: string) {
    const updated = await renameWriterTemplate(templateId, name);
    setTemplates((current) =>
      current.map((template) => (template.id === templateId ? updated : template))
    );
  }

  async function remove(template: WriterTemplateView) {
    if (!window.confirm(`Delete ${template.name}? Existing reports are not affected.`)) {
      return;
    }
    setBusy(template.id);
    setActionError(null);
    try {
      await deleteWriterTemplate(template.id);
      setTemplates((current) =>
        current.filter((candidate) => candidate.id !== template.id)
      );
    } catch (reason) {
      setActionError(String(reason));
    } finally {
      setBusy(null);
    }
  }

  return (
    <NavPane
      paneId="writer.templates"
      summary={
        <span className="text-xs text-gray-400">{templates.length} saved</span>
      }
      testId="writer-template-manager"
      contentClassName="border-t border-gray-100 p-4 space-y-4"
    >
        <div className="flex items-start justify-between gap-4">
          <p className="text-xs text-amber-700">
            Upload only redacted templates. Remove names, dates, diagnoses,
            scores, and other client-specific facts before saving a preset.
          </p>
          <button
            type="button"
            onClick={() => void upload()}
            disabled={busy !== null}
            className="shrink-0 px-3 py-1.5 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50"
          >
            {busy === "upload" ? "Uploading…" : "Upload .docx"}
          </button>
        </div>

        {loading ? (
          <div className="flex items-center gap-2 text-sm text-gray-500">
            <Spinner /> Loading writer templates…
          </div>
        ) : templates.length === 0 ? (
          <p className="rounded-lg border border-dashed border-gray-300 p-4 text-center text-sm text-gray-500">
            No writer templates yet.
          </p>
        ) : (
          <div className="divide-y divide-gray-100 rounded-lg border border-gray-200">
            {templates.map((template) => (
              <div key={template.id} className="flex items-center gap-3 p-3">
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded bg-blue-50 text-xs font-bold text-blue-700">
                  W
                </div>
                <div className="min-w-0 flex-1">
                  <EditableName
                    value={template.name}
                    label="writer template"
                    onSave={(name) => rename(template.id, name)}
                    disabled={busy !== null}
                    className="w-full"
                  />
                  <p className="mt-0.5 text-xs text-gray-400">
                    {formatFileSize(template.size)} · uploaded{" "}
                    {formatDateTime(template.uploaded_at)} · used {template.use_count}{" "}
                    time{template.use_count === 1 ? "" : "s"}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => void remove(template)}
                  disabled={busy !== null}
                  aria-label={`Delete ${template.name}`}
                  title="Delete writer template"
                  className="rounded p-1.5 text-gray-400 hover:bg-red-50 hover:text-red-600 disabled:opacity-40"
                >
                  <TrashIcon />
                </button>
              </div>
            ))}
          </div>
        )}

        {error && (
          <ErrorBanner
            message={error}
            onRetry={() => {
              setActionError(null);
              void reload();
            }}
            className=""
          />
        )}
    </NavPane>
  );
}

function WriterPromptsSection() {
  const { prompts, setPrompts, loading, error: loadError, reload } = useWriterPrompts();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  /** null = closed, "new" = creating, otherwise the id being edited. */
  const [editing, setEditing] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [draftBody, setDraftBody] = useState("");
  const error = loadError ?? actionError;

  function openEditor(prompt: WriterPrompt | null) {
    setEditing(prompt?.id ?? "new");
    setDraftName(prompt?.name ?? "");
    setDraftBody(prompt?.body ?? "");
    setActionError(null);
  }

  async function saveDraft() {
    setBusy(true);
    setActionError(null);
    try {
      const saved = await saveWriterLibraryPrompt(
        editing === "new" ? null : editing,
        draftName,
        draftBody
      );
      setPrompts((current) => {
        const rest = current.filter((prompt) => prompt.id !== saved.id);
        return [...rest, saved].sort((left, right) =>
          left.name.toLowerCase().localeCompare(right.name.toLowerCase())
        );
      });
      setEditing(null);
    } catch (reason) {
      setActionError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function remove(prompt: WriterPrompt) {
    if (!window.confirm(`Delete the saved prompt "${prompt.name}"?`)) {
      return;
    }
    setBusy(true);
    setActionError(null);
    try {
      await deleteWriterLibraryPrompt(prompt.id);
      setPrompts((current) => current.filter((candidate) => candidate.id !== prompt.id));
      if (editing === prompt.id) setEditing(null);
    } catch (reason) {
      setActionError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <NavPane
      paneId="writer.prompt-library"
      summary={<span className="text-xs text-gray-400">{prompts.length} saved</span>}
      testId="writer-prompt-manager"
      contentClassName="border-t border-gray-100 p-4 space-y-4"
    >
      <div className="flex items-start justify-between gap-4">
        <p className="text-xs text-amber-700">
          Saved prompts are shared across all clients. Write placeholders
          like $DIAGNOSIS instead of client names or other identifying
          details.
        </p>
        <button
          type="button"
          onClick={() => openEditor(null)}
          disabled={busy || editing !== null}
          className="shrink-0 px-3 py-1.5 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50"
        >
          New prompt
        </button>
      </div>

      {editing !== null && (
        <div
          className="rounded-lg border border-blue-200 bg-blue-50/40 p-3 space-y-2"
          data-testid="writer-prompt-editor"
        >
          <input
            type="text"
            value={draftName}
            onChange={(event) => setDraftName(event.target.value)}
            placeholder="Prompt name (e.g. Phase 1 — history sections)"
            aria-label="Saved prompt name"
            className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm"
          />
          <textarea
            value={draftBody}
            onChange={(event) => setDraftBody(event.target.value)}
            placeholder="The instruction to prefill, e.g. Fill in Reason for Referral, Background, and Medical/Social History from the records; skip everything else."
            aria-label="Saved prompt text"
            rows={5}
            className="w-full rounded border border-gray-300 px-2 py-1.5 text-sm font-mono"
          />
          <div className="flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setEditing(null)}
              disabled={busy}
              className="px-3 py-1.5 text-sm text-gray-600 hover:text-gray-900 disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => void saveDraft()}
              disabled={busy || draftName.trim() === "" || draftBody.trim() === ""}
              className="px-3 py-1.5 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50"
            >
              {busy ? "Saving…" : "Save prompt"}
            </button>
          </div>
        </div>
      )}

      {loading ? (
        <div className="flex items-center gap-2 text-sm text-gray-500">
          <Spinner /> Loading saved prompts…
        </div>
      ) : prompts.length === 0 && editing === null ? (
        <p className="rounded-lg border border-dashed border-gray-300 p-4 text-center text-sm text-gray-500">
          No saved prompts yet.
        </p>
      ) : (
        prompts.length > 0 && (
          <div className="divide-y divide-gray-100 rounded-lg border border-gray-200">
            {prompts.map((prompt) => (
              <div key={prompt.id} className="flex items-center gap-3 p-3">
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium text-gray-900">
                    {prompt.name}
                  </p>
                  <p className="mt-0.5 truncate text-xs text-gray-400">
                    {prompt.body}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => openEditor(prompt)}
                  disabled={busy || editing !== null}
                  className="rounded px-2 py-1 text-xs font-medium text-gray-600 hover:bg-gray-100 disabled:opacity-40"
                >
                  Edit
                </button>
                <button
                  type="button"
                  onClick={() => void remove(prompt)}
                  disabled={busy}
                  aria-label={`Delete ${prompt.name}`}
                  title="Delete saved prompt"
                  className="rounded p-1.5 text-gray-400 hover:bg-red-50 hover:text-red-600 disabled:opacity-40"
                >
                  <TrashIcon />
                </button>
              </div>
            ))}
          </div>
        )
      )}

      {error && (
        <ErrorBanner
          message={error}
          onRetry={() => {
            setActionError(null);
            void reload();
          }}
          className=""
        />
      )}
    </NavPane>
  );
}

// ---------------------------------------------------------------------------
// Document writer: configurable agentic-loop guardrails
// ---------------------------------------------------------------------------

type WriterLimits = Required<ReportAuthoringPreferences>;
type WriterLimitField = keyof WriterLimits;

const WRITER_LIMIT_DEFAULTS: WriterLimits = {
  max_tool_rounds: 40,
  max_converse_calls: 50,
  max_tool_uses_per_response: 80,
  max_retained_turns: 200,
};

// The `label` values below are quoted verbatim in the writer's
// guardrail-exhausted error, which tells the clinician which field to raise.
// Renaming one here means renaming it in `claria-report-pipeline`'s
// `TOOL_ROUNDS_FIELD_LABEL` / `CONVERSE_CALLS_FIELD_LABEL`.
const WRITER_LIMIT_FIELDS: Array<{
  key: WriterLimitField;
  label: string;
  description: string;
  min: number;
  max: number;
}> = [
  {
    key: "max_tool_rounds",
    label: "Tool-use rounds per request",
    description:
      "How many times the writer may request tools and receive their results before it must finish.",
    min: 1,
    max: 100,
  },
  {
    key: "max_converse_calls",
    label: "Bedrock calls per request",
    description:
      "Total billed model-call ceiling. Whichever call or tool-round guardrail is reached first stops the request.",
    min: 1,
    max: 101,
  },
  {
    key: "max_tool_uses_per_response",
    label: "Tool calls per response",
    description:
      "Maximum list, read, and proposal calls accepted from one model response.",
    min: 1,
    max: 100,
  },
  {
    key: "max_retained_turns",
    label: "Conversation turns retained",
    description:
      "Completed writer turns kept as context. The 512 KiB context-history ceiling may prune older turns sooner.",
    min: 1,
    max: 200,
  },
];

function normalizeWriterPreferences(
  value: ReportAuthoringPreferences | null | undefined
): WriterLimits {
  return {
    max_tool_rounds:
      value?.max_tool_rounds ?? WRITER_LIMIT_DEFAULTS.max_tool_rounds,
    max_converse_calls:
      value?.max_converse_calls ?? WRITER_LIMIT_DEFAULTS.max_converse_calls,
    max_tool_uses_per_response:
      value?.max_tool_uses_per_response ??
      WRITER_LIMIT_DEFAULTS.max_tool_uses_per_response,
    max_retained_turns:
      value?.max_retained_turns ?? WRITER_LIMIT_DEFAULTS.max_retained_turns,
  };
}

function writerLimitsError(value: WriterLimits): string | null {
  for (const field of WRITER_LIMIT_FIELDS) {
    const input = value[field.key];
    if (!Number.isInteger(input) || input < field.min || input > field.max) {
      return `${field.label} must be a whole number from ${field.min} to ${field.max}.`;
    }
  }
  return null;
}

function ReportAuthoringSection() {
  const [snapshot, setSnapshot] = useState<ConfigInfo | null>(null);
  const [draft, setDraft] = useState<WriterLimits | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const info = await fetchCloudPreferences();
      setSnapshot(info);
      setDraft(normalizeWriterPreferences(info.report_authoring));
    } catch (e) {
      try {
        const info = await loadConfig();
        setSnapshot(info);
        setDraft(normalizeWriterPreferences(info.report_authoring));
      } catch (fallbackError) {
        setLoadError(String(fallbackError ?? e));
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const original = snapshot
    ? normalizeWriterPreferences(snapshot.report_authoring)
    : null;
  const dirty =
    original != null &&
    draft != null &&
    JSON.stringify(original) !== JSON.stringify(draft);
  const validationError = draft ? writerLimitsError(draft) : null;
  const effectiveToolRounds = draft
    ? Math.min(
        draft.max_tool_rounds,
        Math.max(0, draft.max_converse_calls - 1)
      )
    : 0;
  const effectiveModelCalls = draft
    ? Math.min(draft.max_converse_calls, draft.max_tool_rounds + 1)
    : 0;
  const theoreticalToolCalls = draft
    ? effectiveToolRounds * draft.max_tool_uses_per_response
    : 0;

  async function save() {
    if (!snapshot || !draft || validationError) return;
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      // Patch-save: only the writer limits travel, so this section cannot
      // roll back a model, cost, or transcription edit.
      const updated = await savePreferencesPatch({ report_authoring: draft });
      setSnapshot(updated);
      setDraft(normalizeWriterPreferences(updated.report_authoring));
      setSaved(true);
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <NavPane
      paneId="writer.limits"
      summary={
        draft ? (
          <span className="text-xs text-gray-400">
            {draft.max_tool_rounds} rounds · {draft.max_converse_calls} calls
          </span>
        ) : undefined
      }
      contentClassName="border-t border-gray-100 p-4 space-y-4"
    >
        {loading ? (
          <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
            <Spinner />
            <span>Loading document writer limits...</span>
          </div>
        ) : !draft ? (
          <ErrorBanner
            message={loadError ?? "Could not load document writer limits."}
            className=""
          />
        ) : (
          <>
            <div className="bg-amber-50 border border-amber-300 rounded-lg p-3 space-y-1.5">
              <p className="text-sm font-medium text-amber-950">
                Higher limits increase cost and runaway-loop exposure
              </p>
              <p className="text-xs text-amber-900">
                These are spend and runtime guardrails, not targets. With this
                combination, one request may make up to {effectiveModelCalls} billed
                Bedrock calls and theoretically issue{" "}
                {theoreticalToolCalls.toLocaleString()} tool calls. It can run much
                longer, repeatedly read client records, and cost substantially more.
              </p>
              <p className="text-xs text-amber-900">
                Opus is generally reliable, but no model is guaranteed not to
                repeat tools or fail to finish. Writer changes still remain
                proposals until you explicitly accept them.
              </p>
            </div>

            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              {WRITER_LIMIT_FIELDS.map((field) => (
                <label
                  key={field.key}
                  className="block"
                  data-pref-anchor={field.key}
                >
                  <span className="text-sm font-medium text-gray-700">
                    {field.label}
                  </span>
                  <input
                    type="number"
                    min={field.min}
                    max={field.max}
                    step={1}
                    value={draft[field.key]}
                    onChange={(event) => {
                      const next = event.currentTarget.valueAsNumber;
                      setDraft({
                        ...draft,
                        [field.key]: Number.isFinite(next) ? next : 0,
                      });
                      setSaved(false);
                    }}
                    className="mt-1 w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  />
                  <span className="block text-xs text-gray-500 mt-1">
                    {field.description}
                  </span>
                </label>
              ))}
            </div>

            {validationError && (
              <ErrorBanner message={validationError} className="" />
            )}
            {saveError && (
              <ErrorBanner
                message={`Could not save document writer limits: ${saveError}`}
                className=""
              />
            )}

            <div className="pt-2 border-t border-gray-100 flex items-center justify-between gap-3">
              <button
                type="button"
                onClick={() => {
                  setDraft(WRITER_LIMIT_DEFAULTS);
                  setSaved(false);
                }}
                disabled={saving}
                className="px-3 py-1.5 text-sm text-gray-600 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors disabled:opacity-50"
              >
                Restore recommended defaults
              </button>
              <div className="flex items-center gap-2">
                {saved && !dirty && (
                  <span className="text-xs text-green-700">Saved</span>
                )}
                <button
                  type="button"
                  onClick={save}
                  disabled={saving || !dirty || validationError != null}
                  className="px-3 py-1.5 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
                >
                  {saving ? "Saving..." : "Save limits"}
                </button>
              </div>
            </div>
          </>
        )}
    </NavPane>
  );
}

// ---------------------------------------------------------------------------
// Transcription section: language / speaker count / engine / translation
// ---------------------------------------------------------------------------

/**
 * Transcription defaults applied to drag-and-drop audio uploads. The wizard
 * uses these as starting values too, but lets the user override per file.
 *
 * Cross-machine sync: on mount we call `fetchCloudPreferences` to pull the
 * latest values from S3 (so the editing machine sees its own recent changes
 * without an app restart). Edits accumulate in a draft and are patch-saved
 * once when the user leaves the section or the screen goes away
 * (`useSaveOnLeave`) — one preferences-file version per editing burst, not
 * one per click. Only this section's fields travel, so sibling sections are
 * never clobbered.
 */
function TranscriptionSection() {
  // The full set of synced fields, fetched on mount.
  const [snapshot, setSnapshot] = useState<ConfigInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Editable copy of the transcription section only.
  const [draft, setDraft] = useState<TranscriptionPreferences | null>(null);
  const dirty =
    snapshot != null &&
    draft != null &&
    JSON.stringify(snapshot.transcription) !== JSON.stringify(draft);

  // Sync status, shown under the controls.
  const [syncing, setSyncing] = useState(false);
  const [syncError, setSyncError] = useState<string | null>(null);
  const [syncedOnce, setSyncedOnce] = useState(false);

  const sync = useCallback(async (next: TranscriptionPreferences) => {
    setSyncing(true);
    setSyncError(null);
    try {
      // Patch-save: only this section's fields travel, so the model
      // dropdown and Cost Explorer sections can never be rolled back by a
      // stale snapshot here.
      const updated = await savePreferencesPatch({ transcription: next });
      // Advancing the snapshot clears `dirty`.
      setSnapshot(updated);
      setSyncedOnce(true);
    } catch (e) {
      // Also logged through the bridge: a flush finishing after unmount has
      // no UI left to show the banner in.
      logFrontendEvent("error", `transcription preferences save failed: ${e}`);
      setSyncError(String(e));
    } finally {
      setSyncing(false);
    }
  }, []);

  // The draft that still needs saving; null when clean or already flushed.
  const pendingRef = useRef<TranscriptionPreferences | null>(null);
  useEffect(() => {
    pendingRef.current = dirty && draft ? draft : null;
  }, [dirty, draft]);
  const flush = useCallback(() => {
    const pending = pendingRef.current;
    if (!pending) return;
    pendingRef.current = null;
    void sync(pending);
  }, [sync]);
  const { containerRef, onContainerBlur } = useSaveOnLeave(flush);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const info = await fetchCloudPreferences();
      setSnapshot(info);
      setDraft(info.transcription);
    } catch (e) {
      // Fall back to whatever's in local config — fetchCloudPreferences
      // requires a configured SDK; without one we still want to render.
      try {
        const info = await loadConfig();
        setSnapshot(info);
        setDraft(info.transcription);
      } catch (e2) {
        setError(String(e2 ?? e));
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  function handleLanguageChange(value: TranscriptionLanguage) {
    if (!draft) return;
    setDraft({ ...draft, default_language: value });
  }

  function handleSpeakerCountChange(value: number) {
    if (!draft) return;
    setDraft({ ...draft, default_speaker_count: value });
  }

  function handleMedicalToggle(value: boolean) {
    if (!draft) return;
    setDraft({ ...draft, use_medical_for_english: value });
  }

  function handleTranslateToggle(value: boolean) {
    if (!draft) return;
    setDraft({ ...draft, translate_to_english: value });
  }

  return (
    <NavPane
      paneId="transcription.imported-audio"
      summary={
        draft ? (
          <span className="text-xs text-gray-400">
            {labelForLanguage(draft.default_language ?? "english")} ·{" "}
            {draft.default_speaker_count ?? 2}{" "}
            {(draft.default_speaker_count ?? 2) === 1 ? "speaker" : "speakers"}
            {draft.use_medical_for_english ? " · Medical" : ""}
            {draft.translate_to_english ? " · translate" : ""}
          </span>
        ) : undefined
      }
      contentClassName="border-t border-gray-100 p-4 space-y-4"
    >
        {loading ? (
          <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
            <Spinner />
            <span>Loading transcription preferences...</span>
          </div>
        ) : !draft ? (
          <ErrorBanner
            message={error ?? "Could not load preferences."}
            className=""
          />
        ) : (
          <div
            ref={containerRef}
            onBlur={onContainerBlur}
            className="space-y-4"
          >
            {/* Language */}
            <fieldset data-pref-anchor="default_language">
              <legend className="text-sm font-medium text-gray-700 mb-2">
                Default language
              </legend>
              <div className="space-y-1.5">
                {(["english", "spanish", "mixed"] as TranscriptionLanguage[]).map(
                  (lang) => (
                    <label
                      key={lang}
                      className="flex items-start gap-2.5 cursor-pointer"
                    >
                      <input
                        type="radio"
                        name="default-language"
                        checked={draft.default_language === lang}
                        onChange={() => handleLanguageChange(lang)}
                        className="mt-0.5"
                      />
                      <div>
                        <span className="text-sm text-gray-900">
                          {labelForLanguage(lang)}
                        </span>
                        <p className="text-xs text-gray-500">
                          {descriptionForLanguage(lang)}
                        </p>
                      </div>
                    </label>
                  )
                )}
              </div>
            </fieldset>

            {/* Speakers */}
            <div data-pref-anchor="default_speaker_count">
              <label className="text-sm font-medium text-gray-700 block mb-1.5">
                Default speakers
              </label>
              <select
                value={draft.default_speaker_count}
                onChange={(e) => handleSpeakerCountChange(Number(e.target.value))}
                className="w-full px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
              >
                <option value={1}>1 — single speaker (no diarization)</option>
                <option value={2}>2 — typical clinician + patient</option>
                <option value={3}>3 — small group</option>
                <option value={4}>4 — family or panel</option>
              </select>
              <p className="text-xs text-gray-500 mt-1">
                Picking "1" turns diarization off and produces a single text
                block (cheaper, faster).
              </p>
            </div>

            {/* Medical toggle */}
            <label
              className="flex items-start gap-2.5 cursor-pointer"
              data-pref-anchor="use_medical_for_english"
            >
              <input
                type="checkbox"
                checked={draft.use_medical_for_english}
                onChange={(e) => handleMedicalToggle(e.target.checked)}
                className="mt-0.5"
              />
              <div>
                <span className="text-sm text-gray-900">
                  Use Transcribe Medical for English sessions
                </span>
                <p className="text-xs text-gray-500">
                  $0.075/min vs Standard $0.024/min — better recognition of
                  clinical vocabulary, drug names, and PHI tagging. Spanish and
                  Mixed sessions always use Standard.
                </p>
              </div>
            </label>

            {/* Translation toggle */}
            <label
              className="flex items-start gap-2.5 cursor-pointer"
              data-pref-anchor="translate_to_english"
            >
              <input
                type="checkbox"
                checked={draft.translate_to_english}
                onChange={(e) => handleTranslateToggle(e.target.checked)}
                className="mt-0.5"
              />
              <div>
                <span className="text-sm text-gray-900">
                  Translate non-English segments to English
                </span>
                <p className="text-xs text-gray-500">
                  When a segment's detected language isn't English, render an
                  English translation alongside the original using your
                  preferred chat model. Adds a few cents per session.
                </p>
              </div>
            </label>

            {syncError ? (
              <ErrorBanner
                message={`Could not save transcription preferences: ${syncError}`}
                onRetry={() => {
                  if (draft) void sync(draft);
                }}
                className=""
              />
            ) : (
              <SectionSaveStatus
                syncing={syncing}
                dirty={dirty}
                syncedOnce={syncedOnce}
              />
            )}
          </div>
        )}
    </NavPane>
  );
}

function labelForLanguage(lang: TranscriptionLanguage): string {
  switch (lang) {
    case "english":
      return "English";
    case "spanish":
      return "Spanish";
    case "mixed":
      return "Mixed (interpreter)";
  }
}

function descriptionForLanguage(lang: TranscriptionLanguage): string {
  switch (lang) {
    case "english":
      return "All English audio. Most common.";
    case "spanish":
      return "All Spanish audio. Always Standard engine.";
    case "mixed":
      return "English and Spanish interleaved. Standard engine, no PHI tagging.";
  }
}


// ---------------------------------------------------------------------------
// Draft runs — the plan gate and the two supporting model roles
// ---------------------------------------------------------------------------

const PLAN_GATE_OPTIONS: {
  value: PlanGateMode;
  label: string;
  description: string;
}[] = [
  {
    value: "gated",
    label: "Show the section plan for review before drafting (recommended)",
    description:
      "The plan lands in the Draft run pane, where scope, evidence, and skipped sections can be changed before a single word is written.",
  },
  {
    value: "auto_start",
    label: "Start drafting as soon as the plan is ready",
    description:
      "No stop between planning and writing. The plan is still shown beside the progress, and the run can still be stopped.",
  },
];

/**
 * Everything about how a whole-report draft is planned.
 *
 * The two model pickers name the supporting roles only — the writer keeps its
 * own picker beside the draft — and leaving either on the default lets Claria
 * resolve it at call time, so an account that gains a better model gets it
 * without anyone editing a setting.
 */
function DraftRunsSection() {
  const {
    models: chatModels,
    loading: modelsLoading,
    error: modelsError,
  } = useChatModels();
  const [snapshot, setSnapshot] = useState<DraftPipelinePreferences | null>(
    null
  );
  const [draft, setDraft] = useState<DraftPipelinePreferences | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [syncedOnce, setSyncedOnce] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const info = await fetchCloudPreferences();
      setSnapshot(info.draft_pipeline);
      setDraft(info.draft_pipeline);
    } catch (e) {
      try {
        const info = await loadConfig();
        setSnapshot(info.draft_pipeline);
        setDraft(info.draft_pipeline);
      } catch (fallbackError) {
        setLoadError(String(fallbackError ?? e));
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const dirty =
    snapshot != null &&
    draft != null &&
    JSON.stringify(snapshot) !== JSON.stringify(draft);

  const sync = useCallback(async (next: DraftPipelinePreferences) => {
    setSyncing(true);
    setSaveError(null);
    try {
      // Patch-save: only the drafting-pipeline fields travel, so this section
      // cannot roll back a model, cost, or writer-limits edit.
      const updated = await savePreferencesPatch({ draft_pipeline: next });
      setSnapshot(updated.draft_pipeline);
      setSyncedOnce(true);
    } catch (e) {
      logFrontendEvent("error", `draft run preferences save failed: ${e}`);
      setSaveError(String(e));
    } finally {
      setSyncing(false);
    }
  }, []);

  const pendingRef = useRef<DraftPipelinePreferences | null>(null);
  useEffect(() => {
    pendingRef.current = dirty && draft ? draft : null;
  }, [dirty, draft]);
  const flush = useCallback(() => {
    const pending = pendingRef.current;
    if (!pending) return;
    pendingRef.current = null;
    void sync(pending);
  }, [sync]);
  const { containerRef, onContainerBlur } = useSaveOnLeave(flush);

  const gate = draft?.plan_gate ?? "gated";
  const active = PLAN_GATE_OPTIONS.find((option) => option.value === gate);

  return (
    <NavPane
      paneId="writer.draft-runs"
      summary={
        active ? (
          <span className="text-xs text-gray-400">
            {gate === "gated" ? "Plan reviewed first" : "Drafts immediately"}
          </span>
        ) : undefined
      }
      contentClassName="border-t border-gray-100 p-4 space-y-4"
    >
      {loading ? (
        <div className="flex items-center gap-2 text-gray-500 text-sm py-2">
          <Spinner />
          <span>Loading draft run settings...</span>
        </div>
      ) : draft == null ? (
        <ErrorBanner
          message={loadError ?? "Could not load the draft run settings."}
          className=""
        />
      ) : (
        <div ref={containerRef} onBlur={onContainerBlur} className="space-y-4">
          <fieldset className="space-y-3" data-pref-anchor="plan_gate">
            <legend className="text-xs font-medium text-gray-700">
              Before drafting starts
            </legend>
            {PLAN_GATE_OPTIONS.map((option) => (
              <label key={option.value} className="flex items-start gap-2">
                <input
                  type="radio"
                  name="plan-gate"
                  value={option.value}
                  checked={gate === option.value}
                  onChange={() =>
                    setDraft({ ...draft, plan_gate: option.value })
                  }
                  className="mt-0.5"
                />
                <span>
                  <span className="text-sm text-gray-700 block">
                    {option.label}
                  </span>
                  <span className="text-xs text-gray-500 block">
                    {option.description}
                  </span>
                </span>
              </label>
            ))}
          </fieldset>

          <div className="space-y-3">
            <div data-pref-anchor="planner_model_id">
              <label className="text-xs text-gray-600" htmlFor="planner-model">
                Planning model
              </label>
              <p className="text-xs text-gray-500">
                Reads the records and decides what each section should cover.
              </p>
              <ModelSelect
                models={chatModels}
                loading={modelsLoading}
                error={modelsError}
                value={draft.planner_model_id ?? ""}
                onChange={(modelId) =>
                  setDraft({
                    ...draft,
                    planner_model_id: modelId === "" ? null : modelId,
                  })
                }
                ariaLabel="Planning model"
                className="mt-1 w-full"
                defaultOption
              />
            </div>
            <div data-pref-anchor="reviewer_model_id">
              <label className="text-xs text-gray-600" htmlFor="reviewer-model">
                Review model
              </label>
              <p className="text-xs text-gray-500">
                Reads the finished draft back and raises findings against it.
              </p>
              <ModelSelect
                models={chatModels}
                loading={modelsLoading}
                error={modelsError}
                value={draft.reviewer_model_id ?? ""}
                onChange={(modelId) =>
                  setDraft({
                    ...draft,
                    reviewer_model_id: modelId === "" ? null : modelId,
                  })
                }
                ariaLabel="Review model"
                className="mt-1 w-full"
                defaultOption
              />
            </div>
          </div>

          {saveError ? (
            <ErrorBanner
              message={`Could not save draft run settings: ${saveError}`}
              onRetry={() => {
                if (draft) void sync(draft);
              }}
              className=""
            />
          ) : (
            <SectionSaveStatus
              syncing={syncing}
              dirty={dirty}
              syncedOnce={syncedOnce}
            />
          )}
        </div>
      )}
    </NavPane>
  );
}

// ---------------------------------------------------------------------------
// Pane registry — every PaneId maps to the component that renders it. A
// component serving several panes (shared state) appears once per pane and
// is mounted once by CategoryPanes. The page test asserts every pane in
// PREFERENCES_NAV actually mounts.
// ---------------------------------------------------------------------------

const PANE_COMPONENTS: Record<PaneId, ComponentType> = {
  "claude.preferred-model": PreferredModelSection,
  "claude.chat-streaming": ChatStreamingSection,
  "claude.model-tuning": ModelTuningSection,
  "prompts.chat-system": ChatSystemPromptSection,
  "prompts.pdf-extraction": PdfExtractionPromptSection,
  "writer.prompt": WriterPromptEditors,
  "writer.whole-report": WriterPromptEditors,
  "writer.prompt-library": WriterPromptsSection,
  "writer.templates": WriterTemplatesSection,
  "writer.draft-runs": DraftRunsSection,
  "writer.limits": ReportAuthoringSection,
  "transcription.imported-audio": TranscriptionSection,
  "transcription.local-models": LocalTranscriptionPanes,
  "transcription.local-compute": LocalTranscriptionPanes,
  "transcription.local-decoding": LocalTranscriptionPanes,
  "billing.cost-explorer": CostExplorerSection,
};
