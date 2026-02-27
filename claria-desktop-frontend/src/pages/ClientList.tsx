import { useState, useEffect, useCallback } from "react";
import { listClients, createClient, type ClientSummary } from "../lib/tauri";
import type { Page } from "../App";

export default function ClientList({
  navigate,
  onOpenClient,
}: {
  navigate: (page: Page) => void;
  onOpenClient: (id: string, name: string) => void;
}) {
  const [clients, setClients] = useState<ClientSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // New client form state
  const [showNewForm, setShowNewForm] = useState(false);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await listClients();
      setClients(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function handleCreate() {
    if (!newName.trim()) return;
    setCreating(true);
    try {
      const created = await createClient(newName.trim());
      setNewName("");
      setShowNewForm(false);
      // Navigate directly to chat for the new client
      onOpenClient(created.id, created.name);
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  }

  return (
    <div className="max-w-2xl mx-auto p-8">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-2xl font-bold">Clients</h2>
        <button
          onClick={() => setShowNewForm(true)}
          className="px-4 py-2 text-sm bg-green-600 text-white rounded-lg hover:bg-green-700 transition-colors"
        >
          New Client
        </button>
      </div>

      {/* New client form */}
      {showNewForm && (
        <div className="bg-white border border-gray-200 rounded-lg p-4 mb-6">
          <h3 className="text-sm font-semibold mb-3">Create New Client</h3>
          <div className="flex gap-3">
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleCreate()}
              placeholder="Client name"
              autoFocus
              className="flex-1 px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-green-500 focus:border-transparent"
            />
            <button
              onClick={handleCreate}
              disabled={creating || !newName.trim()}
              className="px-4 py-2 text-sm bg-green-600 text-white rounded-lg hover:bg-green-700 transition-colors disabled:opacity-50"
            >
              {creating ? "Creating..." : "Create"}
            </button>
            <button
              onClick={() => {
                setShowNewForm(false);
                setNewName("");
              }}
              className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Error */}
      {error && (
        <div className="bg-red-50 border border-red-200 rounded-lg p-4 mb-6">
          <p className="text-red-800 text-sm">{error}</p>
        </div>
      )}

      {/* Loading */}
      {loading && (
        <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 text-center">
          <div className="flex items-center justify-center gap-2 text-blue-800 text-sm">
            <Spinner />
            <span>Loading clients...</span>
          </div>
        </div>
      )}

      {/* Empty state */}
      {!loading && !error && clients.length === 0 && (
        <div className="bg-gray-50 border border-gray-200 rounded-lg p-8 text-center">
          <p className="text-gray-500 text-sm mb-2">No client records yet.</p>
          <p className="text-gray-400 text-xs">
            Click "New Client" to create your first record.
          </p>
        </div>
      )}

      {/* Client table */}
      {!loading && clients.length > 0 && (
        <div className="bg-white border border-gray-200 rounded-lg overflow-hidden">
          <table className="w-full">
            <thead>
              <tr className="border-b border-gray-100 bg-gray-50">
                <th className="text-left text-xs font-medium text-gray-500 px-4 py-2">
                  Name
                </th>
                <th className="text-left text-xs font-medium text-gray-500 px-4 py-2">
                  Date Added
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-100">
              {clients.map((client) => (
                <tr
                  key={client.id}
                  onClick={() => onOpenClient(client.id, client.name)}
                  className="hover:bg-gray-50 cursor-pointer transition-colors"
                >
                  <td className="px-4 py-3 text-sm font-medium text-gray-900">
                    {client.name}
                  </td>
                  <td className="px-4 py-3 text-sm text-gray-500">
                    {formatDate(client.created_at)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Footer */}
      <div className="mt-6">
        <button
          onClick={() => navigate("start")}
          className="px-4 py-2 text-gray-600 hover:text-gray-800"
        >
          Home
        </button>
      </div>
    </div>
  );
}

function formatDate(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  } catch {
    return iso;
  }
}

function Spinner() {
  return (
    <svg className="animate-spin h-3.5 w-3.5" viewBox="0 0 24 24" fill="none">
      <circle
        className="opacity-25"
        cx="12"
        cy="12"
        r="10"
        stroke="currentColor"
        strokeWidth="4"
      />
      <path
        className="opacity-75"
        fill="currentColor"
        d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
      />
    </svg>
  );
}
