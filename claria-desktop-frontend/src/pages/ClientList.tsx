import { useState, useEffect, useCallback } from "react";
import {
  listClients,
  createClient,
  deleteClient,
  listDeletedClients,
  restoreClient,
  type ClientSummary,
  type DeletedClient,
} from "../lib/tauri";
import { formatDate } from "../lib/format";
import { searchMatches } from "../lib/search";
import { useMoreMode } from "../lib/useMoreMode";
import MoreToggle from "../components/MoreToggle";
import DeletedSection from "../components/DeletedSection";
import { ErrorBanner, LoadingCard, EmptyCard } from "../components/StateCards";
import { BackButton, TrashIcon } from "../components/icons";
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

  // Case- and accent-insensitive substring filter on client name, applied
  // to the live and deleted lists alike.
  const [search, setSearch] = useState("");
  const nameMatches = (name: string) => searchMatches(name, search);
  const filteredClients = clients.filter((c) => nameMatches(c.name));

  // New client form state
  const [showNewForm, setShowNewForm] = useState(false);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);

  // Delete confirmation state
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);

  // More mode (deleted clients)
  const {
    moreMode,
    toggleMoreMode,
    deletedItems: deletedClients,
    deletedLoading,
    restoringKey,
    restore,
  } = useMoreMode<DeletedClient>(listDeletedClients, (e) =>
    setError(String(e)),
  );

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

  async function handleDelete(clientId: string) {
    setDeleting(true);
    try {
      await deleteClient(clientId);
      setConfirmDeleteId(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setDeleting(false);
    }
  }

  return (
    <div className="max-w-2xl mx-auto p-8">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <BackButton onClick={() => navigate("start")} />
          <h2 className="text-2xl font-bold">Clients</h2>
        </div>
        <div className="flex gap-2">
          <input
            type="search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search clients…"
            title="Show clients whose name contains this text"
            className="w-40 px-2 py-1 text-xs border border-gray-300 rounded focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500"
          />
          <MoreToggle
            active={moreMode}
            onClick={toggleMoreMode}
            title={moreMode ? "Hide version history" : "Show version history"}
          />
          <button
            onClick={() => setShowNewForm(true)}
            className="px-4 py-2 text-sm bg-green-600 text-white rounded-lg hover:bg-green-700 transition-colors"
          >
            New Client
          </button>
        </div>
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
      {error && <ErrorBanner message={error} />}

      {/* Loading */}
      {loading && <LoadingCard>Loading clients...</LoadingCard>}

      {/* Empty state */}
      {!loading && !error && clients.length === 0 && (
        <EmptyCard>
          <p className="text-gray-500 text-sm mb-2">No client records yet.</p>
          <p className="text-gray-400 text-xs">
            Click "New Client" to create your first record.
          </p>
        </EmptyCard>
      )}

      {/* No search matches */}
      {!loading && clients.length > 0 && filteredClients.length === 0 && (
        <EmptyCard>
          <p className="text-gray-400 text-sm">
            No clients match &ldquo;{search.trim()}&rdquo;
          </p>
        </EmptyCard>
      )}

      {/* Client table */}
      {!loading && filteredClients.length > 0 && (
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
                <th className="w-10" />
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-100">
              {filteredClients.map((client) => (
                <tr
                  key={client.id}
                  data-client={client.id}
                  onClick={() => onOpenClient(client.id, client.name)}
                  className="hover:bg-gray-50 cursor-pointer transition-colors"
                >
                  <td className="px-4 py-3 text-sm font-medium text-gray-900">
                    {client.name}
                  </td>
                  <td className="px-4 py-3 text-sm text-gray-500">
                    {formatDate(client.created_at)}
                  </td>
                  <td className="px-2 py-3 text-right">
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        setConfirmDeleteId(client.id);
                      }}
                      className="text-gray-400 hover:text-red-600 transition-colors p-1"
                      title="Delete client"
                    >
                      <TrashIcon />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Deleted clients (More mode) */}
      {moreMode && !loading && (
        <div className="mt-6">
          <DeletedSection
            title="Deleted Clients"
            noun="clients"
            loading={deletedLoading}
            items={deletedClients}
            itemKey={(dc) => dc.id}
            primary={(dc) => dc.name}
            subtitle={(dc) =>
              dc.deleted_at ? formatDate(dc.deleted_at) : "Unknown"
            }
            searchTerm={search}
            restoringKey={restoringKey}
            onRestore={(dc) =>
              restore(
                dc.id,
                () => restoreClient(dc.id, dc.version_id),
                (c) => c.id === dc.id,
                refresh,
              )
            }
          />
        </div>
      )}

      {/* Delete confirmation modal */}
      {confirmDeleteId && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-sm w-full mx-4 p-6">
            <h3 className="text-lg font-semibold text-gray-900 mb-2">
              Delete client?
            </h3>
            <p className="text-sm text-gray-600 mb-4">
              This will permanently delete the client and all associated records,
              files, and chat history. This cannot be undone.
            </p>
            <div className="flex justify-end gap-3">
              <button
                onClick={() => setConfirmDeleteId(null)}
                disabled={deleting}
                className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800 disabled:opacity-50"
              >
                Cancel
              </button>
              <button
                onClick={() => handleDelete(confirmDeleteId)}
                disabled={deleting}
                className="px-4 py-2 text-sm text-white bg-red-600 rounded-lg hover:bg-red-700 transition-colors disabled:opacity-50"
              >
                {deleting ? "Deleting..." : "Delete"}
              </button>
            </div>
          </div>
        </div>
      )}

    </div>
  );
}
