import { useState, useEffect } from "react";
import StepIndicator from "../components/StepIndicator";
import {
  loadConfig,
  scanResources,
  previewPlan,
  provision,
  type ConfigInfo,
  type ScanResult,
  type Plan,
  type PlanEntry,
} from "../lib/tauri";
import type { Page } from "../App";

type Phase =
  | "idle"          // Initial state — ready to scan
  | "scanning"      // Calling scanResources
  | "scanned"       // Scan complete, showing results
  | "planning"      // Calling previewPlan
  | "planned"       // Plan ready for review
  | "provisioning"  // Executing provision
  | "done"          // Provision complete
  | "error";        // Something went wrong (can retry)

export default function ScanProvision({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  const [config, setConfig] = useState<ConfigInfo | null>(null);
  const [phase, setPhase] = useState<Phase>("idle");
  const [scanResults, setScanResults] = useState<ScanResult[]>([]);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [executedPlan, setExecutedPlan] = useState<Plan | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    loadConfig().then(setConfig).catch(() => {});
  }, []);

  async function handleScan() {
    setPhase("scanning");
    setError(null);
    setScanResults([]);
    setPlan(null);
    setExecutedPlan(null);
    try {
      const results = await scanResources();
      setScanResults(results);
      setPhase("scanned");
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  }

  async function handlePlan() {
    setPhase("planning");
    setError(null);
    try {
      const p = await previewPlan();
      setPlan(p);
      setPhase("planned");
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  }

  async function handleProvision() {
    setPhase("provisioning");
    setError(null);
    try {
      const p = await provision();
      setExecutedPlan(p);
      setPhase("done");
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  }

  const isWorking =
    phase === "scanning" ||
    phase === "planning" ||
    phase === "provisioning";

  // Check if the plan has any changes
  const planHasChanges = plan
    ? plan.create.length > 0 || plan.modify.length > 0 || plan.delete.length > 0
    : false;

  return (
    <div className="max-w-2xl mx-auto p-8">
      <StepIndicator current={4} />

      <h2 className="text-2xl font-bold mb-6">Step 4: Scan &amp; Provision</h2>

      {/* Phase: idle — prompt to start scanning */}
      {phase === "idle" && (
        <div className="bg-gray-50 border border-gray-200 rounded-lg p-6 text-center">
          <p className="text-gray-700 mb-2">
            Claria will scan your AWS account to check the status of all
            managed resources.
          </p>
          <p className="text-gray-500 text-sm mb-4">
            This is a read-only operation — nothing will be created or modified
            until you explicitly approve a plan.
          </p>
          <button
            onClick={handleScan}
            className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
          >
            Start Scan
          </button>
        </div>
      )}

      {/* Phase: scanning */}
      {phase === "scanning" && (
        <div className="bg-blue-50 border border-blue-200 rounded-lg p-6 text-center">
          <div className="flex items-center justify-center gap-2 text-blue-800">
            <Spinner />
            <span>Scanning AWS resources...</span>
          </div>
        </div>
      )}

      {/* Phase: scanned — show scan results */}
      {(phase === "scanned" ||
        phase === "planning" ||
        phase === "planned" ||
        phase === "provisioning" ||
        phase === "done") &&
        scanResults.length > 0 && (
          <ScanResultsTable results={scanResults} config={config} />
        )}

      {/* After scan, show "Review Plan" button */}
      {phase === "scanned" && (
        <div className="mt-6 text-center">
          <button
            onClick={handlePlan}
            className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
          >
            Review Plan
          </button>
        </div>
      )}

      {/* Phase: planning */}
      {phase === "planning" && (
        <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 mt-6 text-center">
          <div className="flex items-center justify-center gap-2 text-blue-800">
            <Spinner />
            <span>Building provisioning plan...</span>
          </div>
        </div>
      )}

      {/* Phase: planned — show plan for review */}
      {(phase === "planned" || phase === "provisioning" || phase === "done") &&
        plan && <PlanReview plan={plan} />}

      {/* Plan review actions */}
      {phase === "planned" && plan && (
        <div className="mt-6 text-center">
          {planHasChanges ? (
            <div>
              <p className="text-gray-600 text-sm mb-3">
                Review the plan above. Click "Provision" to apply changes.
              </p>
              <button
                onClick={handleProvision}
                className="px-6 py-2 bg-green-600 text-white rounded-lg hover:bg-green-700 transition-colors"
              >
                Provision
              </button>
            </div>
          ) : (
            <div className="bg-green-50 border border-green-200 rounded-lg p-4">
              <p className="text-green-800 text-sm font-medium">
                ✅ All resources are in sync — no changes needed.
              </p>
            </div>
          )}
        </div>
      )}

      {/* Phase: provisioning */}
      {phase === "provisioning" && (
        <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 mt-6 text-center">
          <div className="flex items-center justify-center gap-2 text-blue-800">
            <Spinner />
            <span>Provisioning resources...</span>
          </div>
          <p className="text-blue-600 text-xs mt-2">
            State is saved after each step. If interrupted, re-run to resume.
          </p>
        </div>
      )}

      {/* Phase: done — show execution result */}
      {phase === "done" && executedPlan && (
        <div className="mt-6">
          {executedPlan.create.length === 0 &&
          executedPlan.modify.length === 0 &&
          executedPlan.delete.length === 0 ? (
            <div className="bg-green-50 border border-green-200 rounded-lg p-4">
              <p className="text-green-800 text-sm font-medium">
                ✅ Everything was already in sync. No changes were made.
              </p>
            </div>
          ) : (
            <div className="bg-green-50 border border-green-200 rounded-lg p-4">
              <p className="text-green-800 text-sm font-medium mb-2">
                ✅ Provisioning complete!
              </p>
              <ul className="text-green-700 text-sm space-y-1">
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
        </div>
      )}

      {/* Error display */}
      {error && (
        <div className="bg-red-50 border border-red-200 rounded-lg p-4 mt-6">
          <p className="text-red-800 text-sm font-medium mb-1">
            ❌ Error
          </p>
          <p className="text-red-700 text-sm">{error}</p>
        </div>
      )}

      {/* Navigation */}
      <div className="flex justify-between mt-8">
        <button
          onClick={() => navigate("credentials")}
          disabled={isWorking}
          className="px-4 py-2 text-gray-600 hover:text-gray-800 disabled:opacity-50"
        >
          Back
        </button>
        <div className="flex gap-3">
          {/* Retry / Re-scan button */}
          {(phase === "error" || phase === "done") && (
            <button
              onClick={handleScan}
              className="px-6 py-2 bg-white text-gray-700 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors"
            >
              {phase === "error" ? "Retry Scan" : "Re-scan"}
            </button>
          )}

          {/* No-changes or done → go to dashboard */}
          {(phase === "done" ||
            (phase === "planned" && !planHasChanges)) && (
            <button
              onClick={() => navigate("dashboard")}
              className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
            >
              Go to Dashboard
            </button>
          )}
        </div>
      </div>
    </div>
  );
}


// ── Sub-components ──────────────────────────────────────────────────────────

const RESOURCE_LABELS: Record<string, string> = {
  s3_bucket: "S3 Bucket",
  cloudtrail_trail: "CloudTrail Trail",
  bedrock_model_access: "Bedrock Model Access",
  iam_user: "IAM User",
};

const STATUS_STYLES: Record<string, { label: string; color: string; icon: string }> = {
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

function ScanResultsTable({ results, config }: { results: ScanResult[]; config: ConfigInfo | null }) {
  const identity = config?.account_id
    ? `arn:aws:iam::${config.account_id}:user/claria-admin`
    : null;

  return (
    <div className="border border-gray-200 rounded-lg overflow-hidden mt-6">
      <div className="bg-gray-50 px-4 py-2 border-b border-gray-200 flex items-baseline gap-2">
        <h3 className="text-sm font-semibold text-gray-700">Scan Results</h3>
        {identity && (
          <span className="text-xs font-mono text-gray-400 truncate">
            running as {identity}
          </span>
        )}
      </div>
      <div className="divide-y divide-gray-100">
        {results.map((result, i) => {
          const style = STATUS_STYLES[result.status] ?? STATUS_STYLES.error;
          const label = RESOURCE_LABELS[result.resource_type] ?? result.resource_type;

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
                  <ScanProperties properties={result.properties as Record<string, unknown>} resourceType={result.resource_type} />
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

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
    const pab = properties.public_access_block as Record<string, boolean> | null;
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
    const families = (properties.families as { prefix: string; available: boolean; models: string[]; agreement?: string }[]) ?? [];
    const err = properties.error as string | undefined;

    return (
      <div className="mt-1.5 space-y-1">
        <div className="flex flex-wrap gap-2">
          {families.map((f) => (
            <PropertyBadge
              key={f.prefix}
              label={formatModelFamily(f.prefix)}
              ok={f.available && f.agreement === "accepted"}
              value={
                !f.available
                  ? "not enabled"
                  : f.agreement === "accepted"
                    ? `${f.models.length} ready`
                    : f.agreement === "pending"
                      ? "agreement pending"
                      : `${f.models.length} available`
              }
            />
          ))}
        </div>
        {err && (
          <p className="text-xs text-amber-600">{err}</p>
        )}
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
        ok
          ? "bg-green-100 text-green-700"
          : "bg-amber-100 text-amber-700"
      }`}
    >
      <span className="font-medium">{label}:</span>
      <span>{value}</span>
    </span>
  );
}


const MODEL_FAMILY_LABELS: Record<string, string> = {
  "anthropic.claude-sonnet-4": "Claude Sonnet 4",
  "anthropic.claude-opus-4": "Claude Opus 4",
};

function formatModelFamily(prefix: string): string {
  return MODEL_FAMILY_LABELS[prefix] ?? prefix;
}

const BUCKET_CONFIG: Record<
  string,
  { title: string; color: string; borderColor: string; icon: string; emptyMessage: string }
> = {
  ok: {
    title: "OK",
    color: "text-green-800 bg-green-50",
    borderColor: "border-green-200",
    icon: "✅",
    emptyMessage: "No resources in this category.",
  },
  create: {
    title: "Create",
    color: "text-blue-800 bg-blue-50",
    borderColor: "border-blue-200",
    icon: "🆕",
    emptyMessage: "Nothing to create.",
  },
  modify: {
    title: "Modify",
    color: "text-amber-800 bg-amber-50",
    borderColor: "border-amber-200",
    icon: "🔧",
    emptyMessage: "Nothing to modify.",
  },
  delete: {
    title: "Delete",
    color: "text-red-800 bg-red-50",
    borderColor: "border-red-200",
    icon: "🗑️",
    emptyMessage: "Nothing to delete.",
  },
};

function PlanReview({ plan }: { plan: Plan }) {
  const buckets: { key: string; entries: PlanEntry[] }[] = [
    { key: "create", entries: plan.create },
    { key: "modify", entries: plan.modify },
    { key: "delete", entries: plan.delete },
    { key: "ok", entries: plan.ok },
  ];

  // Filter to only show non-empty buckets (except always show create/modify/delete)
  const visibleBuckets = buckets.filter(
    (b) => b.entries.length > 0 || b.key !== "ok"
  );

  return (
    <div className="mt-6 space-y-3">
      <h3 className="text-sm font-semibold text-gray-700">Provisioning Plan</h3>
      {visibleBuckets.map(({ key, entries }) => {
        const cfg = BUCKET_CONFIG[key];
        return (
          <div
            key={key}
            className={`border ${cfg.borderColor} rounded-lg overflow-hidden`}
          >
            <div className={`px-4 py-2 ${cfg.color} flex items-center gap-2`}>
              <span>{cfg.icon}</span>
              <span className="text-sm font-medium">
                {cfg.title}
                {entries.length > 0 && (
                  <span className="ml-1 opacity-70">({entries.length})</span>
                )}
              </span>
            </div>
            {entries.length > 0 ? (
              <div className="divide-y divide-gray-100">
                {entries.map((entry, i) => (
                  <div key={i} className="px-4 py-2">
                    <div className="flex items-center gap-2">
                      <span className="text-sm text-gray-800">
                        {RESOURCE_LABELS[entry.resource_type] ??
                          entry.resource_type}
                      </span>
                      {entry.resource_id && (
                        <span className="text-xs font-mono text-gray-400">
                          {entry.resource_id}
                        </span>
                      )}
                    </div>
                    <p className="text-xs text-gray-500 mt-0.5">
                      {entry.reason}
                    </p>
                  </div>
                ))}
              </div>
            ) : (
              <div className="px-4 py-2">
                <p className="text-xs text-gray-400">{cfg.emptyMessage}</p>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}


function Spinner() {
  return (
    <svg
      className="animate-spin h-4 w-4"
      viewBox="0 0 24 24"
      fill="none"
    >
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