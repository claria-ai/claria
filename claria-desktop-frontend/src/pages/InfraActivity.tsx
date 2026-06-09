import { useState, useEffect, useCallback } from "react";
import {
  listAccountActivity,
  type ActivityEvent,
  type ActivityQuery,
} from "../lib/tauri";
import type { Page } from "../App";

type Window = "1h" | "24h" | "7d" | "30d" | "90d";
type ReadFilter = "all" | "read" | "write";

const WINDOW_HOURS: Record<Window, number> = {
  "1h": 1,
  "24h": 24,
  "7d": 24 * 7,
  "30d": 24 * 30,
  "90d": 24 * 90,
};

function isoHoursAgo(hours: number): string {
  const ms = Date.now() - hours * 60 * 60 * 1000;
  return new Date(ms).toISOString();
}

function relativeTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;
  const diff = Date.now() - then;
  if (diff < 60_000) return "just now";
  if (diff < 3600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

function firstResource(event: ActivityEvent): string {
  const r = event.resources[0];
  if (!r) return "—";
  const name = r.resource_name ?? "";
  const type = r.resource_type ?? "";
  return name || type || "—";
}

export default function InfraActivity({ navigate }: { navigate: (page: Page) => void }) {
  const [windowKey, setWindowKey] = useState<Window>("24h");
  const [eventNameFilter, setEventNameFilter] = useState("");
  const [readFilter, setReadFilter] = useState<ReadFilter>("all");

  const [events, setEvents] = useState<ActivityEvent[]>([]);
  const [nextToken, setNextToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [selected, setSelected] = useState<ActivityEvent | null>(null);

  const buildQuery = useCallback(
    (token: string | null): ActivityQuery => {
      const hours = WINDOW_HOURS[windowKey];
      return {
        start_time: isoHoursAgo(hours),
        end_time: null,
        event_name: eventNameFilter.trim() || null,
        event_source: null,
        username: null,
        read_only: readFilter === "all" ? null : readFilter === "read",
        next_token: token,
        max_results: 50,
      };
    },
    [windowKey, eventNameFilter, readFilter],
  );

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const page = await listAccountActivity(buildQuery(null));
      setEvents(page.events);
      setNextToken(page.next_token);
    } catch (e) {
      setError(String(e));
      setEvents([]);
      setNextToken(null);
    } finally {
      setLoading(false);
    }
  }, [buildQuery]);

  const loadMore = useCallback(async () => {
    if (!nextToken) return;
    setLoadingMore(true);
    setError(null);
    try {
      const page = await listAccountActivity(buildQuery(nextToken));
      setEvents((prev) => [...prev, ...page.events]);
      setNextToken(page.next_token);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoadingMore(false);
    }
  }, [buildQuery, nextToken]);

  // Auto-refresh whenever filters change.
  useEffect(() => {
    load();
  }, [load]);

  return (
    <div className="flex flex-col h-screen">
      {/* Header */}
      <div className="flex items-center gap-3 px-6 py-4 border-b border-gray-200 bg-white">
        <button
          onClick={() => navigate("infra-chat")}
          className="text-gray-500 hover:text-gray-700 transition-colors"
          aria-label="Back"
        >
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M15 19l-7-7 7-7"
            />
          </svg>
        </button>
        <div className="flex-1">
          <h2 className="text-lg font-semibold">Activity</h2>
          <p className="text-xs text-gray-400">
            Recent API calls against your AWS account (CloudTrail, last 90 days)
          </p>
        </div>
      </div>

      {/* Toolbar */}
      <div className="flex items-center gap-3 px-6 py-3 border-b border-gray-100 bg-white flex-wrap">
        <div className="inline-flex rounded-lg border border-gray-200 overflow-hidden text-xs">
          {(Object.keys(WINDOW_HOURS) as Window[]).map((w) => (
            <button
              key={w}
              onClick={() => setWindowKey(w)}
              className={`px-3 py-1.5 transition-colors ${
                windowKey === w
                  ? "bg-blue-50 text-blue-700 font-medium"
                  : "bg-white text-gray-600 hover:bg-gray-50"
              }`}
            >
              {w}
            </button>
          ))}
        </div>

        <input
          type="text"
          value={eventNameFilter}
          onChange={(e) => setEventNameFilter(e.target.value)}
          placeholder="Event name (exact, e.g. PutObject)"
          className="text-xs px-3 py-1.5 border border-gray-200 rounded-lg w-64 focus:outline-none focus:border-blue-400"
        />

        <div className="inline-flex rounded-lg border border-gray-200 overflow-hidden text-xs">
          {(["all", "read", "write"] as ReadFilter[]).map((r) => (
            <button
              key={r}
              onClick={() => setReadFilter(r)}
              className={`px-3 py-1.5 transition-colors ${
                readFilter === r
                  ? "bg-blue-50 text-blue-700 font-medium"
                  : "bg-white text-gray-600 hover:bg-gray-50"
              }`}
            >
              {r}
            </button>
          ))}
        </div>

        <button
          onClick={load}
          disabled={loading}
          className="text-xs px-3 py-1.5 text-gray-600 hover:text-gray-900 disabled:opacity-50"
        >
          {loading ? "Refreshing…" : "Refresh"}
        </button>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto px-6 py-4">
        {error && (
          <div className="bg-red-50 border border-red-200 rounded-lg p-4 mb-4">
            <p className="text-red-800 text-sm">{error}</p>
          </div>
        )}

        {loading && events.length === 0 ? (
          <p className="text-center text-gray-400 text-sm py-12">Loading activity…</p>
        ) : !loading && events.length === 0 && !error ? (
          <p className="text-center text-gray-400 text-sm py-12">
            No events match these filters in the selected window.
          </p>
        ) : (
          <div className="border border-gray-200 rounded-lg overflow-hidden bg-white">
            <table className="w-full text-sm">
              <thead className="bg-gray-50 border-b border-gray-200 text-xs text-gray-500 uppercase tracking-wide">
                <tr>
                  <th className="text-left px-3 py-2 font-medium w-24">When</th>
                  <th className="text-left px-3 py-2 font-medium">Event</th>
                  <th className="text-left px-3 py-2 font-medium">Source</th>
                  <th className="text-left px-3 py-2 font-medium">User</th>
                  <th className="text-left px-3 py-2 font-medium">Resource</th>
                  <th className="text-right px-3 py-2 font-medium w-20"></th>
                </tr>
              </thead>
              <tbody>
                {events.map((event, idx) => (
                  <tr
                    key={event.event_id ?? `${idx}-${event.event_name ?? "unknown"}`}
                    className="border-t border-gray-100 hover:bg-gray-50"
                  >
                    <td
                      className="px-3 py-2 text-gray-500 whitespace-nowrap"
                      title={event.event_time ?? ""}
                    >
                      {relativeTime(event.event_time)}
                    </td>
                    <td className="px-3 py-2 font-mono text-xs text-gray-900">
                      {event.event_name ?? "—"}
                      {event.read_only === false && (
                        <span className="ml-2 text-[10px] uppercase tracking-wide text-amber-700 bg-amber-50 border border-amber-200 rounded px-1 py-0.5">
                          write
                        </span>
                      )}
                    </td>
                    <td className="px-3 py-2 text-gray-600 text-xs">
                      {event.event_source ?? "—"}
                    </td>
                    <td className="px-3 py-2 text-gray-600 text-xs">
                      {event.username ?? "—"}
                    </td>
                    <td className="px-3 py-2 text-gray-600 text-xs truncate max-w-xs">
                      {firstResource(event)}
                    </td>
                    <td className="px-3 py-2 text-right">
                      <button
                        onClick={() => setSelected(event)}
                        className="text-xs text-blue-600 hover:text-blue-800"
                      >
                        View
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {nextToken && (
          <div className="flex justify-center mt-4">
            <button
              onClick={loadMore}
              disabled={loadingMore}
              className="text-xs px-4 py-2 border border-gray-200 rounded-lg text-gray-600 hover:bg-gray-50 disabled:opacity-50"
            >
              {loadingMore ? "Loading…" : "Load more"}
            </button>
          </div>
        )}
      </div>

      {/* Detail modal */}
      {selected && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
          onClick={() => setSelected(null)}
        >
          <div
            className="bg-white rounded-xl shadow-lg max-w-3xl w-full mx-4 p-6 max-h-[80vh] flex flex-col"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between mb-4">
              <div>
                <h3 className="text-lg font-semibold text-gray-900 font-mono">
                  {selected.event_name ?? "Event"}
                </h3>
                <p className="text-xs text-gray-500">
                  {selected.event_time ?? ""} · {selected.event_source ?? ""}
                </p>
              </div>
              <button
                onClick={() => setSelected(null)}
                className="text-gray-400 hover:text-gray-600 transition-colors"
                aria-label="Close"
              >
                <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M6 18L18 6M6 6l12 12"
                  />
                </svg>
              </button>
            </div>
            <div className="flex-1 overflow-y-auto border border-gray-200 rounded-lg p-4 bg-gray-50">
              <pre className="text-xs text-gray-700 whitespace-pre-wrap font-mono">
                {selected.cloudtrail_event_json
                  ? formatJson(selected.cloudtrail_event_json)
                  : "(no event payload)"}
              </pre>
            </div>
            <div className="flex justify-end mt-4">
              <button
                onClick={() => setSelected(null)}
                className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function formatJson(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}
