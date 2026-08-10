import { useEffect, useRef, useState } from "react";
import Modal from "./Modal";
import Spinner from "./Spinner";
import { formatDateTime, formatFileSize } from "../lib/format";
import { diffLines, type DiffLine } from "../lib/diff";
import { useAsyncLoad } from "../lib/useAsyncLoad";
import type { VersionSource } from "../lib/versions";

/**
 * Version history for one S3 object: list, inline preview, restore, and —
 * where the caller asks for it — a two-way diff.
 *
 * The source is captured on mount and the list is fetched once, which matches
 * how both call sites use it: the modal is conditionally rendered, so opening
 * it is a mount and closing it is an unmount.
 */
export default function VersionHistoryModal({
  title,
  source,
  onClose,
  onRestored,
  onError,
  enableCompare = false,
  showFooterClose = false,
  className = "max-w-2xl p-6 max-h-[80vh] flex flex-col",
}: {
  title: string;
  source: VersionSource;
  onClose: () => void;
  /** Called after a successful restore, so the caller can reload its copy. */
  onRestored: () => Promise<void> | void;
  onError: (message: string) => void;
  /** Show the checkboxes and the diff panel. */
  enableCompare?: boolean;
  /** Show a Close button below the list, in addition to the heading's X. */
  showFooterClose?: boolean;
  /** Card sizing classes for the underlying modal. */
  className?: string;
}) {
  const [preview, setPreview] = useState<{
    versionId: string;
    text: string;
  } | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [diffResult, setDiffResult] = useState<DiffLine[] | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [restoring, setRestoring] = useState(false);

  // Both are read from callbacks that outlive the render that created them.
  const sourceRef = useRef(source);
  const onErrorRef = useRef(onError);
  useEffect(() => {
    sourceRef.current = source;
    onErrorRef.current = onError;
  });

  // Fetch once per open — the modal is conditionally rendered, so opening it
  // is a mount. Stale results after close are dropped by the hook.
  const { data, loading, error: listError } = useAsyncLoad(
    () => sourceRef.current.list(),
    []
  );
  const versions = data ?? [];
  useEffect(() => {
    if (listError) onErrorRef.current(listError);
  }, [listError]);

  async function handleView(versionId: string) {
    if (preview?.versionId === versionId) {
      setPreview(null);
      return;
    }
    setPreviewLoading(true);
    try {
      const text = await sourceRef.current.getText(versionId);
      setPreview({ versionId, text });
    } catch (e) {
      // Inline rather than on the caller's banner: the banner sits behind the
      // open dialog, where the user who just clicked View cannot see it.
      setPreview({ versionId, text: `Error: ${String(e)}` });
    } finally {
      setPreviewLoading(false);
    }
  }

  function handleToggleSelect(versionId: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(versionId)) {
        next.delete(versionId);
      } else {
        if (next.size >= 2) {
          // Replace the oldest selection
          const [first] = next;
          next.delete(first);
        }
        next.add(versionId);
      }
      return next;
    });
    setDiffResult(null);
  }

  async function handleCompare() {
    if (selected.size !== 2) return;
    setDiffLoading(true);
    setDiffResult(null);
    try {
      const [v1, v2] = [...selected];
      const [text1, text2] = await Promise.all([
        sourceRef.current.getText(v1),
        sourceRef.current.getText(v2),
      ]);
      // Order by version position: v1 is older, v2 is newer
      const idx1 = versions.findIndex((v) => v.version_id === v1);
      const idx2 = versions.findIndex((v) => v.version_id === v2);
      const [older, newer] = idx1 > idx2 ? [text1, text2] : [text2, text1];
      setDiffResult(diffLines(older, newer));
    } catch (e) {
      onErrorRef.current(String(e));
    } finally {
      setDiffLoading(false);
    }
  }

  async function handleRestore(versionId: string) {
    setRestoring(true);
    try {
      await sourceRef.current.restore(versionId);
      onClose();
      await onRestored();
    } catch (e) {
      onErrorRef.current(String(e));
    } finally {
      setRestoring(false);
    }
  }

  return (
    <Modal open onClose={onClose} title={title} className={className}>
      {loading ? (
        <div className="flex-1 flex items-center justify-center py-8">
          <div className="flex items-center gap-2 text-gray-500 text-sm">
            <Spinner />
            <span>Loading versions...</span>
          </div>
        </div>
      ) : versions.length === 0 ? (
        <div className="flex-1 flex items-center justify-center py-8">
          <p className="text-gray-400 text-sm">No version history found.</p>
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto">
          {enableCompare && (
            <div className="flex items-center justify-between mb-3">
              <p className="text-xs text-gray-500">
                {selected.size === 2
                  ? "2 versions selected"
                  : `Select 2 versions to compare (${selected.size}/2)`}
              </p>
              <button
                onClick={handleCompare}
                disabled={selected.size !== 2 || diffLoading}
                className="px-3 py-1 text-xs font-medium text-white bg-blue-600 rounded hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {diffLoading ? "Comparing..." : "Compare"}
              </button>
            </div>
          )}

          <div className="border border-gray-200 rounded-lg divide-y divide-gray-100">
            {versions.map((v) => (
              <div key={v.version_id}>
                <div className="px-4 py-3 flex items-center gap-3">
                  {enableCompare && (
                    <input
                      type="checkbox"
                      checked={selected.has(v.version_id)}
                      onChange={() => handleToggleSelect(v.version_id)}
                      className="rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                    />
                  )}
                  <div className="flex-1 min-w-0">
                    <p className="text-sm text-gray-900">
                      {v.last_modified
                        ? formatDateTime(v.last_modified)
                        : "Unknown date"}
                      {v.is_latest && (
                        <span className="ml-2 px-1.5 py-0.5 text-xs bg-green-100 text-green-700 rounded">
                          Current
                        </span>
                      )}
                    </p>
                    <p className="text-xs text-gray-400">
                      {formatFileSize(v.size)} &middot;{" "}
                      {v.version_id.slice(0, 12)}...
                    </p>
                  </div>
                  <div className="flex gap-1">
                    <button
                      onClick={() => handleView(v.version_id)}
                      className={`px-2 py-1 text-xs rounded transition-colors ${
                        preview?.versionId === v.version_id
                          ? "bg-blue-100 text-blue-700"
                          : "text-blue-600 hover:bg-blue-50"
                      }`}
                    >
                      {previewLoading && preview?.versionId !== v.version_id
                        ? "..."
                        : preview?.versionId === v.version_id
                          ? "Hide"
                          : "View"}
                    </button>
                    {!v.is_latest && (
                      <button
                        onClick={() => handleRestore(v.version_id)}
                        disabled={restoring}
                        className="px-2 py-1 text-xs text-amber-600 hover:bg-amber-50 rounded transition-colors disabled:opacity-50"
                      >
                        {restoring ? "..." : "Restore"}
                      </button>
                    )}
                  </div>
                </div>
                {/* Inline version preview */}
                {preview?.versionId === v.version_id && (
                  <div className="px-4 pb-3">
                    <pre className="text-xs text-gray-700 whitespace-pre-wrap font-mono bg-gray-50 border border-gray-200 rounded p-3 max-h-[200px] overflow-y-auto">
                      {preview.text}
                    </pre>
                  </div>
                )}
              </div>
            ))}
          </div>

          {diffResult && (
            <div className="mt-4">
              <h4 className="text-sm font-semibold text-gray-700 mb-2">Diff</h4>
              <div className="border border-gray-200 rounded-lg overflow-auto max-h-[20rem]">
                <pre className="text-xs font-mono p-3 whitespace-pre w-max min-w-full">
                  {diffResult.map((line, i) => (
                    <div
                      key={i}
                      className={
                        line.type === "add"
                          ? "bg-green-50 text-green-800"
                          : line.type === "remove"
                            ? "bg-red-50 text-red-800"
                            : "text-gray-600"
                      }
                    >
                      <span className="select-none inline-block w-4 text-gray-400 mr-2">
                        {line.type === "add"
                          ? "+"
                          : line.type === "remove"
                            ? "-"
                            : " "}
                      </span>
                      {line.spans
                        ? line.spans.map((span, si) => (
                            <span
                              key={si}
                              className={
                                span.highlight
                                  ? line.type === "add"
                                    ? "bg-green-200 rounded-sm"
                                    : "bg-red-200 rounded-sm"
                                  : ""
                              }
                            >
                              {span.text}
                            </span>
                          ))
                        : line.line}
                    </div>
                  ))}
                </pre>
              </div>
            </div>
          )}
        </div>
      )}

      {showFooterClose && (
        <div className="flex justify-end mt-4">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
          >
            Close
          </button>
        </div>
      )}
    </Modal>
  );
}
