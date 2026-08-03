import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";
import {
  getClientRecordDetails,
  updateClientName,
  type ClientRecordDetails,
} from "../lib/tauri";
import { formatDate, formatDateTime, formatFileSize } from "../lib/format";
import { ErrorBanner, LoadingCard } from "../components/StateCards";

export default function ClientRecordSettings({
  clientId,
  initialName,
  onNameChanged,
}: {
  clientId: string;
  initialName: string;
  onNameChanged: (name: string) => void;
}) {
  const [details, setDetails] = useState<ClientRecordDetails | null>(null);
  const [name, setName] = useState(initialName);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const loadGenerationRef = useRef(0);
  const onNameChangedRef = useRef(onNameChanged);
  useEffect(() => {
    onNameChangedRef.current = onNameChanged;
  });

  const load = useCallback(async () => {
    const generation = loadGenerationRef.current + 1;
    loadGenerationRef.current = generation;
    setLoading(true);
    setLoadError(null);
    try {
      const result = await getClientRecordDetails(clientId);
      if (loadGenerationRef.current !== generation) return;
      setDetails(result);
      setName(result.name);
      onNameChangedRef.current(result.name);
    } catch (error) {
      if (loadGenerationRef.current === generation) {
        setLoadError(String(error));
      }
    } finally {
      if (loadGenerationRef.current === generation) setLoading(false);
    }
  }, [clientId]);

  useEffect(() => {
    void load();
    return () => {
      loadGenerationRef.current += 1;
    };
  }, [load]);

  async function handleSave(event: FormEvent) {
    event.preventDefault();
    const nextName = name.trim();
    if (!nextName || saving) return;

    setSaving(true);
    setSaved(false);
    setSaveError(null);
    try {
      const updated = await updateClientName(clientId, nextName);
      setName(updated.name);
      setDetails((current) =>
        current
          ? {
              ...current,
              name: updated.name,
              updated_at: updated.updated_at,
              name_history: [
                { name: updated.name, changed_at: updated.updated_at },
                ...current.name_history,
              ],
            }
          : current
      );
      onNameChanged(updated.name);
      setSaved(true);
    } catch (error) {
      setSaveError(String(error));
    } finally {
      setSaving(false);
    }
  }

  const unchanged = details?.name === name.trim();

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-2xl mx-auto p-8">
        <div className="mb-6">
          <h3 className="text-2xl font-bold text-gray-900">Record settings</h3>
          <p className="mt-1 text-sm text-gray-500">
            Update this record&apos;s name and review its storage usage.
          </p>
        </div>

        {loadError && (
          <div>
            <ErrorBanner message={loadError} />
            <button
              type="button"
              onClick={() => void load()}
              className="mb-6 px-3 py-1.5 text-sm font-medium text-blue-700 bg-blue-50 border border-blue-200 rounded-lg hover:bg-blue-100 transition-colors"
            >
              Retry
            </button>
          </div>
        )}
        {saveError && <ErrorBanner message={saveError} />}
        {loading && <LoadingCard>Loading record details...</LoadingCard>}

        {details && !loading && (
          <div className="space-y-6">
            <section className="bg-white border border-gray-200 rounded-xl p-6">
              <form onSubmit={handleSave}>
                <label
                  htmlFor="client-record-name"
                  className="block text-sm font-medium text-gray-700 mb-1.5"
                >
                  Record name
                </label>
                <div className="flex gap-3">
                  <input
                    id="client-record-name"
                    type="text"
                    value={name}
                    maxLength={200}
                    onChange={(event) => {
                      setName(event.target.value);
                      setSaved(false);
                    }}
                    disabled={saving}
                    className="flex-1 px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:bg-gray-50"
                  />
                  <button
                    type="submit"
                    disabled={saving || !name.trim() || unchanged}
                    className="px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    {saving ? "Saving..." : "Save"}
                  </button>
                </div>
                {saved && (
                  <p role="status" className="mt-2 text-sm text-green-700">
                    Record name updated.
                  </p>
                )}
              </form>
            </section>

            <section>
              <div className="grid grid-cols-3 gap-4">
                <StatCard
                  label="Files"
                  value={`${details.file_count} ${
                    details.file_count === 1 ? "file" : "files"
                  }`}
                />
                <StatCard
                  label="Current storage"
                  value={formatFileSize(details.storage_bytes)}
                  secondary={`Including history: ${formatFileSize(
                    details.storage_bytes_with_history
                  )}`}
                />
                <StatCard
                  label="Created"
                  value={formatDate(details.created_at)}
                />
              </div>
              <p className="mt-3 text-xs text-gray-400">
                Storage includes files, extracted text, Chat history, and
                Writing data.
              </p>
            </section>

            <p className="text-xs text-gray-400">
              Record ID: <span className="font-mono">{details.id}</span>
            </p>

            <section>
              <h4 className="mb-3 text-sm font-semibold text-gray-900">
                Name history
              </h4>
              <div className="overflow-hidden bg-white border border-gray-200 rounded-xl">
                <table
                  aria-label="Record name history"
                  className="w-full text-sm"
                >
                  <thead className="bg-gray-50 text-left text-xs font-medium uppercase tracking-wide text-gray-400">
                    <tr>
                      <th scope="col" className="px-4 py-3">
                        Name
                      </th>
                      <th scope="col" className="px-4 py-3">
                        Time
                      </th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-gray-100">
                    {details.name_history.length === 0 ? (
                      <tr>
                        <td
                          colSpan={2}
                          className="px-4 py-4 text-gray-400"
                        >
                          No name history is available.
                        </td>
                      </tr>
                    ) : (
                      details.name_history.map((entry, index) => (
                        <tr key={`${entry.changed_at}-${index}`}>
                          <td className="px-4 py-3 font-medium text-gray-900">
                            {entry.name}
                          </td>
                          <td className="px-4 py-3 text-gray-500">
                            <time dateTime={entry.changed_at}>
                              {formatDateTime(entry.changed_at)}
                            </time>
                          </td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </div>
            </section>
          </div>
        )}
      </div>
    </div>
  );
}

function StatCard({
  label,
  value,
  secondary,
}: {
  label: string;
  value: string;
  secondary?: string;
}) {
  return (
    <div className="bg-white border border-gray-200 rounded-xl p-4">
      <p className="text-xs font-medium uppercase tracking-wide text-gray-400">
        {label}
      </p>
      <p className="mt-2 text-lg font-semibold text-gray-900">{value}</p>
      {secondary && (
        <p className="mt-1 text-xs text-gray-400">{secondary}</p>
      )}
    </div>
  );
}
