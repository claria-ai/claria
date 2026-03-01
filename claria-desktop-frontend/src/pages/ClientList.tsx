import { useState, useEffect, useCallback } from "react";
import {
  listClients,
  createClient,
  deleteClient,
  getSystemPrompt,
  saveSystemPrompt,
  deleteSystemPrompt,
  type ClientSummary,
} from "../lib/tauri";
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

  // Delete confirmation state
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);

  // System prompt editor state
  const [showPromptEditor, setShowPromptEditor] = useState(false);
  const [promptContent, setPromptContent] = useState("");
  const [promptLoading, setPromptLoading] = useState(false);
  const [promptSaving, setPromptSaving] = useState(false);
  const [promptError, setPromptError] = useState<string | null>(null);

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

  async function handleOpenPromptEditor() {
    setShowPromptEditor(true);
    setPromptLoading(true);
    setPromptError(null);
    try {
      const content = await getSystemPrompt();
      setPromptContent(content);
    } catch (e) {
      setPromptError(String(e));
    } finally {
      setPromptLoading(false);
    }
  }

  async function handleSavePrompt() {
    setPromptSaving(true);
    setPromptError(null);
    try {
      await saveSystemPrompt(promptContent);
      setShowPromptEditor(false);
    } catch (e) {
      setPromptError(String(e));
    } finally {
      setPromptSaving(false);
    }
  }

  async function handleResetPrompt() {
    setPromptSaving(true);
    setPromptError(null);
    try {
      await deleteSystemPrompt();
      const content = await getSystemPrompt();
      setPromptContent(content);
    } catch (e) {
      setPromptError(String(e));
    } finally {
      setPromptSaving(false);
    }
  }

  return (
    <div className="max-w-2xl mx-auto p-8">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <button
            onClick={() => navigate("start")}
            className="text-gray-500 hover:text-gray-700 transition-colors"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
            </svg>
          </button>
          <h2 className="text-2xl font-bold">Clients</h2>
        </div>
        <div className="flex gap-2">
          <button
            onClick={handleOpenPromptEditor}
            className="px-4 py-2 text-sm text-gray-600 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors"
          >
            System Prompt
          </button>
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
                <th className="w-10" />
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
                  <td className="px-2 py-3 text-right">
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        setConfirmDeleteId(client.id);
                      }}
                      className="text-gray-400 hover:text-red-600 transition-colors p-1"
                      title="Delete client"
                    >
                      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                      </svg>
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
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

      {/* System prompt editor modal */}
      {showPromptEditor && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="bg-white rounded-xl shadow-lg max-w-2xl w-full mx-4 p-6 max-h-[80vh] flex flex-col">
            <h3 className="text-lg font-semibold text-gray-900 mb-4">
              System Prompt
            </h3>

            {promptLoading ? (
              <div className="flex-1 flex items-center justify-center py-8">
                <div className="flex items-center gap-2 text-gray-500 text-sm">
                  <Spinner />
                  <span>Loading prompt...</span>
                </div>
              </div>
            ) : (
              <textarea
                value={promptContent}
                onChange={(e) => setPromptContent(e.target.value)}
                disabled={promptSaving}
                className="flex-1 min-h-[300px] px-3 py-2 text-sm font-mono border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent resize-y disabled:bg-gray-50"
              />
            )}

            {promptError && (
              <div className="bg-red-50 border border-red-200 rounded-lg p-3 mt-3">
                <p className="text-red-800 text-sm">{promptError}</p>
              </div>
            )}

            <div className="flex justify-between mt-4">
              <button
                onClick={handleResetPrompt}
                disabled={promptLoading || promptSaving}
                className="px-4 py-2 text-sm text-amber-600 border border-amber-300 rounded-lg hover:bg-amber-50 transition-colors disabled:opacity-50"
              >
                {promptSaving ? "Resetting..." : "Reset to Default"}
              </button>
              <div className="flex gap-3">
                <button
                  onClick={() => setShowPromptEditor(false)}
                  disabled={promptSaving}
                  className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800 disabled:opacity-50"
                >
                  Cancel
                </button>
                <button
                  onClick={handleSavePrompt}
                  disabled={promptLoading || promptSaving}
                  className="px-4 py-2 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50"
                >
                  {promptSaving ? "Saving..." : "Save"}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
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
