import { useEffect, useState } from "react";

/** Compact inline name editor shared by chat and writer session headers. */
export default function EditableName({
  value,
  label,
  onSave,
  disabled = false,
  className = "",
  compactActions = false,
}: {
  value: string;
  label: string;
  onSave: (name: string) => Promise<void>;
  disabled?: boolean;
  className?: string;
  compactActions?: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!editing) setDraft(value);
  }, [editing, value]);

  function cancel() {
    setDraft(value);
    setError(null);
    setEditing(false);
  }

  async function save() {
    const name = draft.trim();
    if (!name || saving) return;
    if (name === value) {
      setEditing(false);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await onSave(name);
      setEditing(false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  }

  if (!editing) {
    return (
      <button
        type="button"
        onClick={() => setEditing(true)}
        disabled={disabled}
        aria-label={`Rename ${label}`}
        title={`${value} — rename ${label}`}
        className={`group inline-flex min-w-0 items-center gap-1.5 text-left disabled:cursor-default ${className}`}
      >
        <span className="truncate font-semibold text-gray-900">{value}</span>
        {!disabled && (
          <span
            aria-hidden="true"
            className="text-[11px] text-gray-300 opacity-0 transition-opacity group-hover:opacity-100 group-focus:opacity-100"
          >
            ✎
          </span>
        )}
      </button>
    );
  }

  return (
    <div className={`min-w-0 ${className}`}>
      <div className="flex items-center gap-1.5">
        <input
          autoFocus
          aria-label={`${label} name`}
          value={draft}
          maxLength={120}
          disabled={saving}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void save();
            } else if (event.key === "Escape") {
              event.preventDefault();
              cancel();
            }
          }}
          className="min-w-0 flex-1 rounded-md border border-blue-300 bg-white px-2 py-1 text-sm font-semibold text-gray-900 focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        <button
          type="button"
          onClick={() => void save()}
          disabled={saving || draft.trim() === ""}
          aria-label={compactActions ? `Save ${label} name` : undefined}
          title={compactActions ? "Save" : undefined}
          className="text-xs font-medium text-blue-700 disabled:opacity-40"
        >
          {saving ? "Saving…" : compactActions ? "✓" : "Save"}
        </button>
        <button
          type="button"
          onClick={cancel}
          disabled={saving}
          aria-label={compactActions ? `Cancel ${label} rename` : undefined}
          title={compactActions ? "Cancel" : undefined}
          className="text-xs text-gray-500 disabled:opacity-40"
        >
          {compactActions ? "×" : "Cancel"}
        </button>
      </div>
      {error && <p className="mt-1 text-xs text-red-600">{error}</p>}
    </div>
  );
}
