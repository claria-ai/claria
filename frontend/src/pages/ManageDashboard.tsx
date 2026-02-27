import { useState, useEffect } from "react";
import { loadConfig, deleteConfig, type ConfigInfo } from "../lib/tauri";
import type { Page } from "../App";

export default function ManageDashboard({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  const [config, setConfig] = useState<ConfigInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);

  useEffect(() => {
    loadConfig()
      .then(setConfig)
      .catch((e) => setError(String(e)));
  }, []);

  async function handleDelete() {
    if (!confirm("Delete your Claria config? This cannot be undone.")) return;
    setDeleting(true);
    try {
      await deleteConfig();
      navigate("start");
    } catch (e) {
      setError(String(e));
      setDeleting(false);
    }
  }

  if (error && !config) {
    return (
      <div className="max-w-2xl mx-auto p-8">
        <h2 className="text-2xl font-bold mb-6">Dashboard</h2>
        <div className="bg-red-50 border border-red-200 rounded-lg p-4">
          <p className="text-red-800 text-sm">{error}</p>
        </div>
        <button
          onClick={() => navigate("start")}
          className="mt-4 px-4 py-2 text-gray-600 hover:text-gray-800"
        >
          Back
        </button>
      </div>
    );
  }

  if (!config) {
    return (
      <div className="max-w-2xl mx-auto p-8">
        <p className="text-gray-500">Loading config...</p>
      </div>
    );
  }

  return (
    <div className="max-w-2xl mx-auto p-8">
      <h2 className="text-2xl font-bold mb-6">Dashboard</h2>

      <div className="bg-white border border-gray-200 rounded-lg p-6 mb-6">
        <h3 className="text-lg font-semibold mb-4">System Configuration</h3>
        <dl className="space-y-3">
          <div className="flex justify-between">
            <dt className="text-sm text-gray-500">Region</dt>
            <dd className="text-sm font-mono">{config.region}</dd>
          </div>
          <div className="flex justify-between">
            <dt className="text-sm text-gray-500">System Name</dt>
            <dd className="text-sm font-mono">{config.system_name}</dd>
          </div>
          <div className="flex justify-between">
            <dt className="text-sm text-gray-500">Credential Type</dt>
            <dd className="text-sm font-mono">{config.credential_type}</dd>
          </div>
          {config.profile_name && (
            <div className="flex justify-between">
              <dt className="text-sm text-gray-500">Profile</dt>
              <dd className="text-sm font-mono">{config.profile_name}</dd>
            </div>
          )}
          {config.access_key_hint && (
            <div className="flex justify-between">
              <dt className="text-sm text-gray-500">Access Key</dt>
              <dd className="text-sm font-mono">{config.access_key_hint}</dd>
            </div>
          )}
          <div className="flex justify-between">
            <dt className="text-sm text-gray-500">Created</dt>
            <dd className="text-sm">{config.created_at}</dd>
          </div>
        </dl>
      </div>

      <div className="bg-gray-50 border border-gray-200 rounded-lg p-6 mb-6">
        <h3 className="text-lg font-semibold mb-2">Resource Status</h3>
        <p className="text-gray-500 text-sm">
          Provisioner integration coming soon. Once connected, this section will
          show the status of all managed AWS resources.
        </p>
      </div>

      <div className="flex gap-3">
        <button
          onClick={() => navigate("start")}
          className="px-4 py-2 text-gray-600 hover:text-gray-800"
        >
          Home
        </button>
        <div className="flex-1" />
        <button
          onClick={handleDelete}
          disabled={deleting}
          className="px-4 py-2 text-red-600 border border-red-300 rounded-lg hover:bg-red-50 transition-colors disabled:opacity-50"
        >
          {deleting ? "Deleting..." : "Delete Config"}
        </button>
      </div>
    </div>
  );
}
