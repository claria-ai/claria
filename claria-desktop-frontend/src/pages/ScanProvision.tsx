import { useState } from "react";
import StepIndicator from "../components/StepIndicator";
import PlanView, { hasChanges } from "../components/PlanView";
import { plan, apply, resetProvisionerState, type PlanEntry } from "../lib/tauri";
import type { Page } from "../App";

type Phase =
  | "idle"
  | "scanning"
  | "planned"
  | "applying"
  | "done"
  | "error";

export default function ScanProvision({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  const [entries, setEntries] = useState<PlanEntry[] | null>(null);
  const [phase, setPhase] = useState<Phase>("idle");
  const [error, setError] = useState<string | null>(null);
  const [resettingState, setResettingState] = useState(false);

  async function handleScan() {
    setPhase("scanning");
    setError(null);
    try {
      setEntries(await plan());
      setPhase("planned");
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  }

  async function handleApply() {
    setPhase("applying");
    setError(null);
    try {
      setEntries(await apply());
      setPhase("done");
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  }

  const isWorking = phase === "scanning" || phase === "applying";

  return (
    <div className="max-w-2xl mx-auto p-8">
      <StepIndicator current={4} />

      <h2 className="text-2xl font-bold mb-6">Step 4: Review &amp; Provision</h2>

      {/* Phase: idle — prompt to start scanning */}
      {phase === "idle" && (
        <div className="bg-gray-50 border border-gray-200 rounded-lg p-6 text-center">
          <p className="text-gray-700 mb-2">
            Claria will scan your AWS account to check the status of all
            managed resources.
          </p>
          <p className="text-gray-500 text-sm mb-4">
            This is a read-only operation — nothing will be created or modified
            until you explicitly approve.
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

      {/* Plan view */}
      {entries && <PlanView entries={entries} />}

      {/* Plan actions */}
      {phase === "planned" && entries && (
        <div className="mt-6 text-center">
          {hasChanges(entries) ? (
            <div>
              <p className="text-gray-600 text-sm mb-3">
                Review the plan above. Click "Apply Changes" to proceed.
              </p>
              <button
                onClick={handleApply}
                className="px-6 py-2 bg-green-600 text-white rounded-lg hover:bg-green-700 transition-colors"
              >
                Apply Changes
              </button>
            </div>
          ) : (
            <div className="bg-green-50 border border-green-200 rounded-lg p-4">
              <p className="text-green-800 text-sm font-medium">
                All resources are in sync — no changes needed.
              </p>
            </div>
          )}
        </div>
      )}

      {/* Phase: applying */}
      {phase === "applying" && (
        <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 mt-6 text-center">
          <div className="flex items-center justify-center gap-2 text-blue-800">
            <Spinner />
            <span>Applying changes...</span>
          </div>
          <p className="text-blue-600 text-xs mt-2">
            State is saved after each step. If interrupted, re-run to resume.
          </p>
        </div>
      )}

      {/* Phase: done */}
      {phase === "done" && (
        <div className="bg-green-50 border border-green-200 rounded-lg p-4 mt-6">
          <p className="text-green-800 text-sm font-medium">
            Provisioning complete!
          </p>
        </div>
      )}

      {/* Error display */}
      {error && (
        <div className="mt-6">
          <div className="bg-red-50 border border-red-200 rounded-lg p-4">
            <p className="text-red-800 text-sm font-medium mb-1">Error</p>
            <p className="text-red-700 text-sm">{error}</p>
          </div>
          {error.includes("incompatible") && (
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
                    setError(null);
                    handleScan();
                  } catch (e) {
                    setError(String(e));
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
          {(phase === "error" || phase === "done") && (
            <button
              onClick={handleScan}
              className="px-6 py-2 bg-white text-gray-700 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors"
            >
              {phase === "error" ? "Retry Scan" : "Re-scan"}
            </button>
          )}
          {(phase === "done" ||
            (phase === "planned" && !hasChanges(entries))) && (
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
