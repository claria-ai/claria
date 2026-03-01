import { useState, useEffect, useCallback } from "react";
import {
  loadConfig,
  deleteConfig,
  plan,
  apply,
  destroy,
  resetProvisionerState,
  escalateIamPolicy,
  type ConfigInfo,
  type PlanEntry,
} from "../lib/tauri";
import PlanView, { hasChanges } from "../components/PlanView";
import type { Page } from "../App";

type ResourcePhase =
  | "idle"
  | "scanning"
  | "planned"
  | "applying"
  | "applied"
  | "destroying"
  | "destroyed";

export default function ManageDashboard({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  const [config, setConfig] = useState<ConfigInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [showDestroyConfirm, setShowDestroyConfirm] = useState(false);
  const [resettingState, setResettingState] = useState(false);
  const [showEscalation, setShowEscalation] = useState(false);

  // Resource status state
  const [resourcePhase, setResourcePhase] = useState<ResourcePhase>("idle");
  const [entries, setEntries] = useState<PlanEntry[] | null>(null);
  const [resourceError, setResourceError] = useState<string | null>(null);

  useEffect(() => {
    loadConfig()
      .then(setConfig)
      .catch((e) => setError(String(e)));
  }, []);

  const handleScan = useCallback(async () => {
    setResourcePhase("scanning");
    setResourceError(null);
    try {
      setEntries(await plan());
      setResourcePhase("planned");
    } catch (e) {
      setResourceError(String(e));
      setResourcePhase("idle");
    }
  }, []);

  // Auto-scan on load once config is available.
  const [didInitialScan, setDidInitialScan] = useState(false);
  useEffect(() => {
    if (config && !didInitialScan) {
      setDidInitialScan(true);
      void handleScan();
    }
  }, [config, didInitialScan, handleScan]);

  async function handleApply() {
    setResourcePhase("applying");
    setResourceError(null);
    try {
      setEntries(await apply());
      setResourcePhase("applied");
      // Re-scan after a short delay to show updated state
      setTimeout(() => {
        handleScan();
      }, 1000);
    } catch (e) {
      setResourceError(String(e));
      setResourcePhase("planned");
    }
  }

  async function handleDestroy() {
    setResourcePhase("destroying");
    setResourceError(null);
    setShowDestroyConfirm(false);
    try {
      await destroy();
      setResourcePhase("destroyed");
      setEntries(null);
    } catch (e) {
      setResourceError(String(e));
      setResourcePhase("planned");
    }
  }

  async function handleDeleteConfig() {
    setDeleting(true);
    try {
      await deleteConfig();
      navigate("start");
    } catch (e) {
      setError(String(e));
      setDeleting(false);
      setShowDeleteConfirm(false);
    }
  }

  const isWorking =
    resourcePhase === "scanning" ||
    resourcePhase === "applying" ||
    resourcePhase === "destroying";

  if (error && !config) {
    return (
      <div className="max-w-2xl mx-auto p-8">
        <h2 className="text-2xl font-bold mb-6">Dashboard</h2>
        <div className="bg-red-50 border border-red-200 rounded-lg p-4">
          <p className="text-red-800 font-medium text-sm mb-2">
            Failed to load configuration
          </p>
          <p className="text-red-700 text-xs font-mono whitespace-pre-wrap">{error}</p>
        </div>
        <div className="bg-amber-50 border border-amber-200 rounded-lg p-4 mt-4">
          <p className="text-amber-800 text-sm">
            Your config file may be corrupt or incompatible with this version of
            Claria. You can clear it and start fresh.
          </p>
        </div>
        <div className="flex gap-3 mt-4">
          <button
            onClick={() => navigate("start")}
            className="px-4 py-2 text-gray-600 hover:text-gray-800"
          >
            Back
          </button>
          <button
            onClick={async () => {
              setDeleting(true);
              try {
                await deleteConfig();
                navigate("start");
              } catch (e) {
                setError(String(e));
                setDeleting(false);
              }
            }}
            disabled={deleting}
            className="px-4 py-2 text-sm text-white bg-red-600 rounded-lg hover:bg-red-700 transition-colors disabled:opacity-50"
          >
            {deleting ? "Clearing..." : "Clear Config & Start Over"}
          </button>
        </div>
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

      {/* System Configuration */}
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

      {/* Infrastructure */}
      <div className="bg-white border border-gray-200 rounded-lg p-6 mb-6">
        <div className="flex items-center justify-between mb-4">
          <div>
            <h3 className="text-lg font-semibold">Infrastructure</h3>
            {config.account_id && (
              <p className="text-xs font-mono text-gray-400 mt-0.5">
                running as arn:aws:iam::{config.account_id}:user/claria-admin
              </p>
            )}
          </div>
          <button
            onClick={handleScan}
            disabled={isWorking}
            className="px-3 py-1.5 text-xs font-medium text-gray-600 bg-gray-100 rounded-lg hover:bg-gray-200 transition-colors disabled:opacity-50"
          >
            {resourcePhase === "scanning" ? (
              <span className="flex items-center gap-1">
                <Spinner /> Scanning...
              </span>
            ) : (
              "Re-scan"
            )}
          </button>
        </div>

        {/* Scanning indicator (when no results yet) */}
        {resourcePhase === "scanning" && !entries && (
          <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 text-center">
            <div className="flex items-center justify-center gap-2 text-blue-800 text-sm">
              <Spinner />
              <span>Scanning AWS resources...</span>
            </div>
          </div>
        )}

        {/* Plan view */}
        {entries && (
          <PlanView
            entries={entries}
            onEscalate={() => setShowEscalation(true)}
          />
        )}

        {/* Apply button */}
        {entries && hasChanges(entries) && resourcePhase === "planned" && (
          <div className="mt-4 flex items-center gap-3">
            <button
              onClick={handleApply}
              disabled={isWorking}
              className="px-4 py-2 text-sm bg-green-600 text-white rounded-lg hover:bg-green-700 transition-colors disabled:opacity-50"
            >
              Apply Changes
            </button>
            <span className="text-xs text-gray-500">
              Apply changes to bring resources in sync.
            </span>
          </div>
        )}

        {/* Applied result */}
        {resourcePhase === "applied" && (
          <div className="bg-green-50 border border-green-200 rounded-lg p-4 mt-4">
            <p className="text-green-800 text-sm font-medium">
              Changes applied successfully.
            </p>
          </div>
        )}

        {/* Destroyed state */}
        {resourcePhase === "destroyed" && (
          <div className="bg-amber-50 border border-amber-200 rounded-lg p-4 mt-4">
            <p className="text-amber-800 text-sm font-medium">
              All managed resources have been destroyed.
            </p>
            <p className="text-amber-700 text-xs mt-1">
              Click "Re-scan" to verify, or re-provision from the setup wizard.
            </p>
          </div>
        )}

        {/* Resource error */}
        {resourceError && (
          <div className="mt-4">
            <div className="bg-red-50 border border-red-200 rounded-lg p-4">
              <p className="text-red-800 text-sm">{resourceError}</p>
            </div>
            {resourceError.includes("incompatible") && (
              <div className="bg-amber-50 border border-amber-200 rounded-lg p-4 mt-3">
                <p className="text-amber-800 text-sm">
                  The provisioner state file is not compatible with this version
                  of Claria. You can reset it and re-scan — your AWS resources
                  are not affected.
                </p>
                <button
                  onClick={async () => {
                    setResettingState(true);
                    try {
                      await resetProvisionerState();
                      setResourceError(null);
                      handleScan();
                    } catch (e) {
                      setResourceError(String(e));
                    } finally {
                      setResettingState(false);
                    }
                  }}
                  disabled={resettingState}
                  className="mt-3 px-4 py-2 text-sm text-white bg-amber-600 rounded-lg hover:bg-amber-700 transition-colors disabled:opacity-50"
                >
                  {resettingState ? "Resetting..." : "Reset State & Re-scan"}
                </button>
              </div>
            )}
          </div>
        )}
      </div>

      {/* General error */}
      {error && (
        <div className="bg-red-50 border border-red-200 rounded-lg p-4 mb-6">
          <p className="text-red-800 text-sm">{error}</p>
        </div>
      )}

      {/* Actions */}
      <div className="flex gap-3">
        <button
          onClick={() => navigate("start")}
          className="px-4 py-2 text-gray-600 hover:text-gray-800"
        >
          Home
        </button>
        <div className="flex-1" />
        <button
          onClick={() => setShowDestroyConfirm(true)}
          disabled={isWorking}
          className="px-4 py-2 text-red-600 border border-red-300 rounded-lg hover:bg-red-50 transition-colors disabled:opacity-50"
        >
          Destroy Resources
        </button>
        <button
          onClick={() => setShowDeleteConfirm(true)}
          disabled={isWorking}
          className="px-4 py-2 text-red-600 border border-red-300 rounded-lg hover:bg-red-50 transition-colors disabled:opacity-50"
        >
          Delete Config
        </button>
      </div>

      {/* Delete config confirmation dialog */}
      {showDeleteConfirm && (
        <ConfirmDialog
          title="Delete Configuration?"
          body="This will remove your local Claria configuration including saved
            credentials. Your AWS resources will not be affected. This cannot
            be undone."
          confirmLabel={deleting ? "Deleting..." : "Delete"}
          confirmDisabled={deleting}
          onCancel={() => setShowDeleteConfirm(false)}
          onConfirm={handleDeleteConfig}
        />
      )}

      {/* Destroy resources confirmation dialog */}
      {showDestroyConfirm && (
        <ConfirmDialog
          title="Destroy All Resources?"
          body="This will delete the S3 bucket (including all stored data),
            the CloudTrail trail, and clear the provisioner state. Your local
            config will be kept so you can re-provision later. This cannot be undone."
          confirmLabel="Destroy"
          confirmDisabled={false}
          onCancel={() => setShowDestroyConfirm(false)}
          onConfirm={handleDestroy}
        />
      )}

      {/* IAM policy escalation dialog */}
      {showEscalation && (
        <EscalationDialog
          onCancel={() => setShowEscalation(false)}
          onSuccess={() => {
            setShowEscalation(false);
            handleScan();
          }}
        />
      )}
    </div>
  );
}


// ── Sub-components ──────────────────────────────────────────────────────────

function ConfirmDialog({
  title,
  body,
  confirmLabel,
  confirmDisabled,
  onCancel,
  onConfirm,
}: {
  title: string;
  body: string;
  confirmLabel: string;
  confirmDisabled: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
      <div className="bg-white rounded-xl shadow-lg max-w-sm w-full mx-4 p-6">
        <h3 className="text-lg font-semibold text-gray-900 mb-2">{title}</h3>
        <p className="text-sm text-gray-600 mb-6">{body}</p>
        <div className="flex justify-end gap-3">
          <button
            onClick={onCancel}
            disabled={confirmDisabled}
            className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800 disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={confirmDisabled}
            className="px-4 py-2 text-sm text-white bg-red-600 rounded-lg hover:bg-red-700 transition-colors disabled:opacity-50"
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

function EscalationDialog({
  onCancel,
  onSuccess,
}: {
  onCancel: () => void;
  onSuccess: () => void;
}) {
  const [accessKeyId, setAccessKeyId] = useState("");
  const [secretAccessKey, setSecretAccessKey] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      await escalateIamPolicy(accessKeyId.trim(), secretAccessKey.trim());
      onSuccess();
    } catch (err) {
      setError(String(err));
      setSubmitting(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
      <div className="bg-white rounded-xl shadow-lg max-w-md w-full mx-4 p-6">
        <h3 className="text-lg font-semibold text-gray-900 mb-2">
          Update IAM Policy
        </h3>
        <p className="text-sm text-gray-600 mb-4">
          This Claria update needs additional AWS permissions. Provide your root
          or admin access key to update the IAM policy. These credentials are
          used once and never saved.
        </p>

        <form onSubmit={handleSubmit} className="space-y-3">
          <div>
            <label className="block text-xs font-medium text-gray-700 mb-1">
              Access Key ID
            </label>
            <input
              type="text"
              value={accessKeyId}
              onChange={(e) => setAccessKeyId(e.target.value)}
              placeholder="AKIA..."
              disabled={submitting}
              className="w-full px-3 py-2 text-sm font-mono border border-gray-300 rounded-lg focus:ring-2 focus:ring-amber-500 focus:border-amber-500 disabled:opacity-50"
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-gray-700 mb-1">
              Secret Access Key
            </label>
            <input
              type="password"
              value={secretAccessKey}
              onChange={(e) => setSecretAccessKey(e.target.value)}
              disabled={submitting}
              className="w-full px-3 py-2 text-sm font-mono border border-gray-300 rounded-lg focus:ring-2 focus:ring-amber-500 focus:border-amber-500 disabled:opacity-50"
            />
          </div>

          {error && (
            <div className="bg-red-50 border border-red-200 rounded-lg p-3">
              <p className="text-red-800 text-xs">{error}</p>
            </div>
          )}

          <div className="flex justify-end gap-3 pt-2">
            <button
              type="button"
              onClick={onCancel}
              disabled={submitting}
              className="px-4 py-2 text-sm text-gray-600 hover:text-gray-800 disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={submitting || !accessKeyId.trim() || !secretAccessKey.trim()}
              className="px-4 py-2 text-sm text-white bg-amber-600 rounded-lg hover:bg-amber-700 transition-colors disabled:opacity-50"
            >
              {submitting ? "Updating..." : "Update Policy"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
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
