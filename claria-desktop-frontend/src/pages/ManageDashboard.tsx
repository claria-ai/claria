import { useState, useEffect, useCallback } from "react";
import {
  loadConfig,
  deleteConfig,
  scanResources,
  previewPlan,
  provision,
  destroy,
  type ConfigInfo,
  type ScanResult,
  type Plan,
  type PlanEntry,
} from "../lib/tauri";
import type { Page } from "../App";

type ResourcePhase =
  | "idle"
  | "scanning"
  | "scanned"
  | "planning"
  | "planned"
  | "provisioning"
  | "provisioned"
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

  // Resource status state
  const [resourcePhase, setResourcePhase] = useState<ResourcePhase>("idle");
  const [scanResults, setScanResults] = useState<ScanResult[]>([]);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [executedPlan, setExecutedPlan] = useState<Plan | null>(null);
  const [resourceError, setResourceError] = useState<string | null>(null);

  useEffect(() => {
    loadConfig()
      .then(setConfig)
      .catch((e) => setError(String(e)));
  }, []);

  const handleScan = useCallback(async () => {
    setResourcePhase("scanning");
    setResourceError(null);
    setScanResults([]);
    setPlan(null);
    setExecutedPlan(null);
    try {
      const results = await scanResources();
      setScanResults(results);
      setResourcePhase("scanned");
    } catch (e) {
      setResourceError(String(e));
      setResourcePhase("idle");
    }
  }, []);

  // Auto-scan on load once config is available.
  // We track whether the initial scan has fired to avoid re-triggering
  // on every config reference change.
  const [didInitialScan, setDidInitialScan] = useState(false);
  useEffect(() => {
    if (config && !didInitialScan) {
      setDidInitialScan(true);
      // Fire-and-forget — the scan callback manages its own state via refs.
      void handleScan();
    }
  }, [config, didInitialScan, handleScan]);

  async function handleCheckDrift() {
    setResourcePhase("planning");
    setResourceError(null);
    setPlan(null);
    setExecutedPlan(null);
    try {
      // Re-scan first so scan results are fresh
      const results = await scanResources();
      setScanResults(results);
      const p = await previewPlan();
      setPlan(p);
      setResourcePhase("planned");
    } catch (e) {
      setResourceError(String(e));
      setResourcePhase("scanned");
    }
  }

  async function handleReconcile() {
    setResourcePhase("provisioning");
    setResourceError(null);
    try {
      const p = await provision();
      setExecutedPlan(p);
      setResourcePhase("provisioned");
      // Re-scan after provisioning to show updated state
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
      setScanResults([]);
      setPlan(null);
      setExecutedPlan(null);
    } catch (e) {
      setResourceError(String(e));
      setResourcePhase("scanned");
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
    resourcePhase === "planning" ||
    resourcePhase === "provisioning" ||
    resourcePhase === "destroying";

  const planHasChanges = plan
    ? plan.create.length > 0 || plan.modify.length > 0 || plan.delete.length > 0
    : false;

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

      {/* Resource Status */}
      <div className="bg-white border border-gray-200 rounded-lg p-6 mb-6">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-semibold">Resource Status</h3>
          <div className="flex gap-2">
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
            <button
              onClick={handleCheckDrift}
              disabled={isWorking}
              className="px-3 py-1.5 text-xs font-medium text-blue-600 bg-blue-50 rounded-lg hover:bg-blue-100 transition-colors disabled:opacity-50"
            >
              {resourcePhase === "planning" ? (
                <span className="flex items-center gap-1">
                  <Spinner /> Checking...
                </span>
              ) : (
                "Check for Drift"
              )}
            </button>
          </div>
        </div>

        {/* Scanning indicator (when no results yet) */}
        {resourcePhase === "scanning" && scanResults.length === 0 && (
          <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 text-center">
            <div className="flex items-center justify-center gap-2 text-blue-800 text-sm">
              <Spinner />
              <span>Scanning AWS resources...</span>
            </div>
          </div>
        )}

        {/* Scan results */}
        {scanResults.length > 0 && (
          <div className="divide-y divide-gray-100 border border-gray-100 rounded-lg overflow-hidden">
            {scanResults.map((result, i) => {
              const style = STATUS_STYLES[result.status] ?? STATUS_STYLES.error;
              const label =
                RESOURCE_LABELS[result.resource_type] ?? result.resource_type;

              return (
                <div key={i} className="px-4 py-3 flex items-start gap-3">
                  <span className="shrink-0 mt-0.5">{style.icon}</span>
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-gray-800">
                        {label}
                      </span>
                      <span
                        className={`text-xs px-1.5 py-0.5 rounded ${style.color}`}
                      >
                        {style.label}
                      </span>
                    </div>
                    {result.resource_id && (
                      <p className="text-xs font-mono text-gray-500 mt-0.5 truncate">
                        {result.resource_id}
                      </p>
                    )}
                    {result.error && (
                      <p className="text-xs text-red-600 mt-0.5">
                        {result.error}
                      </p>
                    )}
                    {result.status === "found" && result.properties && (
                      <ScanProperties
                        properties={result.properties as Record<string, unknown>}
                        resourceType={result.resource_type}
                      />
                    )}
                  </div>
                </div>
              );
            })}
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
          <div className="bg-red-50 border border-red-200 rounded-lg p-4 mt-4">
            <p className="text-red-800 text-sm">{resourceError}</p>
          </div>
        )}
      </div>

      {/* Drift / Plan section */}
      {plan && (
        <div className="bg-white border border-gray-200 rounded-lg p-6 mb-6">
          <h3 className="text-lg font-semibold mb-4">Drift Detection</h3>

          {planHasChanges ? (
            <>
              <PlanSummary plan={plan} />
              <div className="mt-4 flex items-center gap-3">
                <button
                  onClick={handleReconcile}
                  disabled={isWorking}
                  className="px-4 py-2 text-sm bg-green-600 text-white rounded-lg hover:bg-green-700 transition-colors disabled:opacity-50"
                >
                  {resourcePhase === "provisioning" ? (
                    <span className="flex items-center gap-1">
                      <Spinner /> Reconciling...
                    </span>
                  ) : (
                    "Reconcile"
                  )}
                </button>
                <span className="text-xs text-gray-500">
                  Apply changes to bring resources in sync.
                </span>
              </div>
            </>
          ) : (
            <div className="bg-green-50 border border-green-200 rounded-lg p-4">
              <p className="text-green-800 text-sm font-medium">
                ✅ All resources are in sync — no drift detected.
              </p>
            </div>
          )}
        </div>
      )}

      {/* Reconcile result */}
      {executedPlan && resourcePhase === "provisioned" && (
        <div className="bg-green-50 border border-green-200 rounded-lg p-4 mb-6">
          <p className="text-green-800 text-sm font-medium mb-1">
            ✅ Reconciliation complete
          </p>
          <ul className="text-green-700 text-xs space-y-0.5">
            {executedPlan.create.length > 0 && (
              <li>
                Created {executedPlan.create.length} resource
                {executedPlan.create.length !== 1 ? "s" : ""}
              </li>
            )}
            {executedPlan.modify.length > 0 && (
              <li>
                Updated {executedPlan.modify.length} resource
                {executedPlan.modify.length !== 1 ? "s" : ""}
              </li>
            )}
            {executedPlan.delete.length > 0 && (
              <li>
                Cleaned up {executedPlan.delete.length} stale entr
                {executedPlan.delete.length !== 1 ? "ies" : "y"}
              </li>
            )}
          </ul>
        </div>
      )}

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
    </div>
  );
}


// ── Sub-components ──────────────────────────────────────────────────────────

const RESOURCE_LABELS: Record<string, string> = {
  s3_bucket: "S3 Bucket",
  cloudtrail_trail: "CloudTrail Trail",
  bedrock_model_access: "Bedrock Model Access",
};

const STATUS_STYLES: Record<
  string,
  { label: string; color: string; icon: string }
> = {
  found: {
    label: "Found",
    color: "text-green-800 bg-green-50",
    icon: "✅",
  },
  not_found: {
    label: "Not Found",
    color: "text-gray-600 bg-gray-50",
    icon: "⬜",
  },
  error: {
    label: "Error",
    color: "text-red-800 bg-red-50",
    icon: "❌",
  },
};

function ScanProperties({
  properties,
  resourceType,
}: {
  properties: Record<string, unknown>;
  resourceType: string;
}) {
  if (resourceType === "s3_bucket") {
    const versioning = properties.versioning as string | null;
    const encryption = properties.encryption as string | null;
    const pab = properties.public_access_block as Record<
      string,
      boolean
    > | null;
    const allBlocked = pab
      ? pab.block_public_acls &&
        pab.ignore_public_acls &&
        pab.block_public_policy &&
        pab.restrict_public_buckets
      : false;

    return (
      <div className="flex flex-wrap gap-2 mt-1.5">
        <PropertyBadge
          label="Versioning"
          ok={versioning === "Enabled"}
          value={versioning ?? "disabled"}
        />
        <PropertyBadge
          label="Encryption"
          ok={!!encryption}
          value={encryption ?? "none"}
        />
        <PropertyBadge
          label="Public Access Block"
          ok={allBlocked}
          value={allBlocked ? "all blocked" : "incomplete"}
        />
      </div>
    );
  }

  if (resourceType === "cloudtrail_trail") {
    const isLogging = properties.is_logging as boolean | null;
    return (
      <div className="flex flex-wrap gap-2 mt-1.5">
        <PropertyBadge
          label="Logging"
          ok={isLogging === true}
          value={isLogging ? "active" : "stopped"}
        />
      </div>
    );
  }

  if (resourceType === "bedrock_model_access") {
    const models = (properties.available_models as string[]) ?? [];
    const err = properties.error as string | undefined;
    if (err) {
      return <p className="text-xs text-amber-600 mt-1">{err}</p>;
    }
    return (
      <div className="flex flex-wrap gap-2 mt-1.5">
        <PropertyBadge
          label="Models"
          ok={models.length > 0}
          value={models.length > 0 ? `${models.length} available` : "none"}
        />
      </div>
    );
  }

  return null;
}

function PropertyBadge({
  label,
  ok,
  value,
}: {
  label: string;
  ok: boolean;
  value: string;
}) {
  return (
    <span
      className={`inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full ${
        ok ? "bg-green-100 text-green-700" : "bg-amber-100 text-amber-700"
      }`}
    >
      <span className="font-medium">{label}:</span>
      <span>{value}</span>
    </span>
  );
}

function PlanSummary({ plan }: { plan: Plan }) {
  const buckets: { key: string; entries: PlanEntry[]; cfg: (typeof BUCKET_CONFIG)[string] }[] = [
    { key: "create", entries: plan.create, cfg: BUCKET_CONFIG.create },
    { key: "modify", entries: plan.modify, cfg: BUCKET_CONFIG.modify },
    { key: "delete", entries: plan.delete, cfg: BUCKET_CONFIG.delete },
  ].filter((b) => b.entries.length > 0);

  if (buckets.length === 0) return null;

  return (
    <div className="space-y-2">
      {buckets.map(({ key, entries, cfg }) => (
        <div
          key={key}
          className={`border ${cfg.borderColor} rounded-lg overflow-hidden`}
        >
          <div className={`px-3 py-1.5 ${cfg.color} flex items-center gap-2`}>
            <span className="text-sm">{cfg.icon}</span>
            <span className="text-xs font-medium">
              {cfg.title} ({entries.length})
            </span>
          </div>
          <div className="divide-y divide-gray-100">
            {entries.map((entry, i) => (
              <div key={i} className="px-3 py-1.5">
                <div className="flex items-center gap-2">
                  <span className="text-xs text-gray-800">
                    {RESOURCE_LABELS[entry.resource_type] ?? entry.resource_type}
                  </span>
                  {entry.resource_id && (
                    <span className="text-xs font-mono text-gray-400">
                      {entry.resource_id}
                    </span>
                  )}
                </div>
                <p className="text-xs text-gray-500">{entry.reason}</p>
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

const BUCKET_CONFIG: Record<
  string,
  { title: string; color: string; borderColor: string; icon: string }
> = {
  create: {
    title: "Create",
    color: "text-blue-800 bg-blue-50",
    borderColor: "border-blue-200",
    icon: "🆕",
  },
  modify: {
    title: "Modify",
    color: "text-amber-800 bg-amber-50",
    borderColor: "border-amber-200",
    icon: "🔧",
  },
  delete: {
    title: "Delete",
    color: "text-red-800 bg-red-50",
    borderColor: "border-red-200",
    icon: "🗑️",
  },
};

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