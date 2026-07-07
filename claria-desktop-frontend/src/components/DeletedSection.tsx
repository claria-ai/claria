import type { ReactNode } from "react";
import Spinner from "./Spinner";

export default function DeletedSection<T>({
  title,
  noun,
  loading,
  items,
  itemKey,
  primary,
  subtitle,
  icon,
  searchTerm,
  restoringKey,
  onRestore,
}: {
  title: string;
  noun: string;
  loading: boolean;
  items: T[];
  itemKey: (item: T) => string;
  primary: (item: T) => string;
  subtitle: (item: T) => string;
  icon?: (item: T) => ReactNode;
  searchTerm?: string;
  restoringKey: string | null;
  onRestore: (item: T) => void;
}) {
  const query = searchTerm?.trim().toLowerCase() ?? "";
  const filtered = query
    ? items.filter((item) => primary(item).toLowerCase().includes(query))
    : items;

  return (
    <>
      <h3 className="text-sm font-semibold text-gray-500 mb-3">{title}</h3>
      {loading ? (
        <div className="bg-gray-50 border border-gray-200 rounded-lg p-4 text-center">
          <div className="flex items-center justify-center gap-2 text-gray-500 text-sm">
            <Spinner />
            <span>Loading deleted {noun}...</span>
          </div>
        </div>
      ) : filtered.length === 0 ? (
        <div className="bg-gray-50 border border-gray-200 rounded-lg p-4 text-center">
          <p className="text-gray-400 text-sm">
            {items.length === 0
              ? `No deleted ${noun} found.`
              : `No deleted ${noun} match “${searchTerm?.trim() ?? ""}”`}
          </p>
        </div>
      ) : (
        <div className="bg-white border border-gray-200 rounded-lg overflow-hidden divide-y divide-gray-100">
          {filtered.map((item) => {
            const key = itemKey(item);
            return (
              <div
                key={key}
                className="px-4 py-3 flex items-center gap-3 opacity-60"
              >
                {icon?.(item)}
                <div className="flex-1 min-w-0">
                  <p className="text-sm text-gray-500 line-through truncate">
                    {primary(item)}
                  </p>
                  <p className="text-xs text-gray-400">{subtitle(item)}</p>
                </div>
                <button
                  onClick={() => onRestore(item)}
                  disabled={restoringKey !== null}
                  className="px-3 py-1 text-xs text-blue-600 border border-blue-300 rounded hover:bg-blue-50 disabled:opacity-50 shrink-0"
                >
                  {restoringKey === key ? "Restoring..." : "Restore"}
                </button>
              </div>
            );
          })}
        </div>
      )}
    </>
  );
}
