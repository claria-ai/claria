import { useState, useEffect, useRef, useCallback, useMemo, type ReactNode } from "react";
import { getConsoleLogs, getConsoleLogsText, saveConsoleLogs } from "../lib/tauri";
import type { ConsoleEntry } from "../lib/tauri";

const LEVELS = ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] as const;

const POLL_INTERVAL_MS = 500;

/** Rows rendered by default; older lines sit behind a "show all" toggle. */
const MAX_RENDERED_ENTRIES = 500;

/**
 * Cheap identity for a log snapshot, used to skip re-rendering an unchanged
 * buffer. Length alone misses the case that matters most here: the backend
 * buffer is bounded, so once it is full every new line rotates one out and the
 * length stops changing. Both ends go into the fingerprint, and either moves
 * on rotation.
 */
function fingerprint(entries: ConsoleEntry[]): string {
  if (entries.length === 0) return "0";
  const first = entries[0];
  const last = entries[entries.length - 1];
  return [
    entries.length,
    first.timestamp,
    first.message,
    last.timestamp,
    last.message,
  ].join("\u001f");
}

function levelColor(level: string): string {
  switch (level) {
    case "ERROR":
      return "text-red-600";
    case "WARN":
      return "text-amber-600";
    case "DEBUG":
      return "text-gray-400";
    case "TRACE":
      return "text-gray-300";
    default:
      return "text-gray-700";
  }
}

function highlightMatch(text: string, query: string): (string | ReactNode)[] {
  if (!query) return [text];
  const lower = text.toLowerCase();
  const q = query.toLowerCase();
  const parts: (string | ReactNode)[] = [];
  let last = 0;
  let idx = lower.indexOf(q, last);
  while (idx !== -1) {
    if (idx > last) parts.push(text.slice(last, idx));
    parts.push(
      <mark key={idx} className="bg-yellow-200 rounded px-0.5">
        {text.slice(idx, idx + query.length)}
      </mark>
    );
    last = idx + query.length;
    idx = lower.indexOf(q, last);
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

export default function Console() {
  const [entries, setEntries] = useState<ConsoleEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [enabledLevels, setEnabledLevels] = useState<Set<string>>(
    () => new Set(LEVELS)
  );
  const [copied, setCopied] = useState(false);
  const logRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const fetchLogs = useCallback(async () => {
    setLoading(true);
    try {
      setEntries(await getConsoleLogs());
    } catch {
      // If fetch fails, keep existing entries
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchLogs();
  }, [fetchLogs]);

  // Poll for new logs twice a second while the window is open.
  const [autoScroll, setAutoScroll] = useState(true);
  useEffect(() => {
    const id = setInterval(async () => {
      try {
        // TODO(#73 PERF-2): replace with a get_console_logs_since(seq) delta
        // command so each poll ships only new lines instead of the whole
        // bounded buffer twice a second.
        const latest = await getConsoleLogs();
        setEntries((prev) =>
          fingerprint(latest) === fingerprint(prev) ? prev : latest
        );
      } catch {
        // Ignore poll failures
      }
    }, POLL_INTERVAL_MS);
    return () => clearInterval(id);
  }, []);

  // Auto-scroll to bottom when new entries arrive (if user hasn't scrolled up).
  useEffect(() => {
    if (autoScroll && logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [entries, autoScroll]);

  // Detect if user has scrolled away from the bottom to pause auto-scroll.
  const handleScroll = useCallback(() => {
    if (!logRef.current) return;
    const { scrollTop, scrollHeight, clientHeight } = logRef.current;
    setAutoScroll(scrollHeight - scrollTop - clientHeight < 40);
  }, []);

  // Cmd+F focuses the search input
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        e.preventDefault();
        searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const filtered = useMemo(() => {
    const q = search.toLowerCase();
    return entries.filter((e) => {
      if (!enabledLevels.has(e.level)) return false;
      if (q) {
        const line = `${e.timestamp} ${e.level} ${e.target}: ${e.message}`;
        if (!line.toLowerCase().includes(q)) return false;
      }
      return true;
    });
  }, [entries, search, enabledLevels]);

  // Render only the newest lines by default — the buffer can hold thousands
  // of rows, and re-rendering them all twice a second is wasted work.
  const [showAll, setShowAll] = useState(false);
  const visible = showAll ? filtered : filtered.slice(-MAX_RENDERED_ENTRIES);
  const hiddenCount = filtered.length - visible.length;

  const toggleLevel = (level: string) => {
    setEnabledLevels((prev) => {
      const next = new Set(prev);
      if (next.has(level)) next.delete(level);
      else next.add(level);
      return next;
    });
  };

  const handleCopy = async () => {
    try {
      const text = await getConsoleLogsText();
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard API may fail in some contexts
    }
  };

  const handleSave = async () => {
    try {
      await saveConsoleLogs();
    } catch {
      // Save may fail
    }
  };

  return (
    <div className="flex flex-col h-screen bg-gray-50">
      {/* Header */}
      <div className="flex items-center gap-3 px-6 py-4 border-b border-gray-200 bg-white">
        <h2 className="text-lg font-semibold">Claria Console</h2>
        <span className="text-xs text-gray-400 ml-1">
          {entries.length} entries
        </span>

        <div className="ml-auto flex items-center gap-2">
          {/* Refresh */}
          <button
            onClick={fetchLogs}
            className="px-3 py-1.5 text-xs font-medium text-gray-600 bg-gray-100 hover:bg-gray-200 rounded transition-colors"
          >
            Refresh
          </button>
          {/* Copy */}
          <button
            onClick={handleCopy}
            className="px-3 py-1.5 text-xs font-medium text-gray-600 bg-gray-100 hover:bg-gray-200 rounded transition-colors"
          >
            {copied ? "Copied!" : "Copy"}
          </button>
          {/* Save As */}
          <button
            onClick={handleSave}
            className="px-3 py-1.5 text-xs font-medium text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors"
          >
            Save As...
          </button>
        </div>
      </div>

      {/* Toolbar: search + level filters */}
      <div className="flex items-center gap-3 px-6 py-2 border-b border-gray-200 bg-white">
        <input
          ref={searchRef}
          type="text"
          placeholder="Search logs... (Cmd+F)"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="flex-1 px-3 py-1.5 text-sm border border-gray-300 rounded focus:outline-none focus:ring-1 focus:ring-blue-500"
        />
        <div className="flex gap-1">
          {LEVELS.map((level) => (
            <button
              key={level}
              onClick={() => toggleLevel(level)}
              className={`px-2 py-1 text-xs font-mono rounded transition-colors ${
                enabledLevels.has(level)
                  ? level === "ERROR"
                    ? "bg-red-100 text-red-700"
                    : level === "WARN"
                      ? "bg-amber-100 text-amber-700"
                      : level === "DEBUG"
                        ? "bg-gray-200 text-gray-500"
                        : level === "TRACE"
                          ? "bg-gray-100 text-gray-400"
                          : "bg-blue-100 text-blue-700"
                  : "bg-gray-100 text-gray-300"
              }`}
            >
              {level}
            </button>
          ))}
        </div>
      </div>

      {/* Log viewer */}
      <div
        ref={logRef}
        onScroll={handleScroll}
        className="flex-1 overflow-auto px-6 py-3 font-mono text-xs leading-5"
      >
        {loading ? (
          <p className="text-gray-400 text-center mt-8">Loading...</p>
        ) : filtered.length === 0 ? (
          <p className="text-gray-400 text-center mt-8">
            {entries.length === 0 ? "No log entries yet." : "No matching entries."}
          </p>
        ) : (
          <>
            {hiddenCount > 0 && (
              <button
                onClick={() => setShowAll(true)}
                className="block w-full py-1.5 mb-1 text-center text-xs text-blue-600 bg-blue-50 hover:bg-blue-100 rounded transition-colors"
              >
                Show all {filtered.length.toLocaleString()} entries (
                {hiddenCount.toLocaleString()} older hidden)
              </button>
            )}
            {visible.map((entry, i) => {
              const line = `${entry.timestamp} ${entry.level} ${entry.target}: ${entry.message}`;
              return (
                <div key={i} className={`${levelColor(entry.level)} whitespace-pre-wrap break-all`}>
                  {search ? highlightMatch(line, search) : line}
                </div>
              );
            })}
          </>
        )}
      </div>
    </div>
  );
}
