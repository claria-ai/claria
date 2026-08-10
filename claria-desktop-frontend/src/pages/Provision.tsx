import { useState, useEffect, useCallback } from "react";
import {
  hasConfig,
  loadConfig,
  deleteConfig,
  assessCredentials,
  assumeRole,
  listAwsProfiles,
  provisionScan,
  provisionApply,
  listUserAccessKeys,
  deleteUserAccessKey,
  destroy,
  resetProvisionerState,
  plan,
  apply,
  type CredentialSource,

  type AccessKeyInfo,
  type AccessKeyLimitReached,
  type AssumeRoleResult,
  type PlanEntry,
  type ConfigInfo,
} from "../lib/tauri";
import AccessKeyLimitPanel from "../components/AccessKeyLimitPanel";
import InfraState from "../components/InfraState";
import { hasChanges, findEscalationEntry } from "../lib/plan";
import { useProvisionerProgress } from "../lib/provisioner";
import type { Page } from "../App";

const AWS_REGIONS = [
  "us-east-1", "us-east-2", "us-west-1", "us-west-2",
  "eu-west-1", "eu-west-2", "eu-west-3", "eu-central-1", "eu-north-1",
  "ap-southeast-1", "ap-southeast-2", "ap-northeast-1", "ap-northeast-2",
  "ap-south-1", "ca-central-1", "sa-east-1",
];

const DEFAULT_ROLE_NAME = "OrganizationAccountAccessRole";

type CredMode = "inline" | "sub_account" | "profile" | "default_chain";

type Phase =
  | "loading"         // Checking if config exists
  | "input"           // Credential entry (first run)
  | "scanning"        // Scanning all resources
  | "planned"         // Plan ready, show results
  | "escalation"      // Need elevated creds, show inline form
  | "applying"        // Executing changes
  | "key_limit"       // IAM is out of access-key slots; operator must free one
  | "done"            // Apply succeeded, showing final state
  | "error";          // Something failed

export default function Provision({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  // ── Config state ─────────────────────────────────────────────────────
  const [configExists, setConfigExists] = useState<boolean | null>(null);
  const [config, setConfig] = useState<ConfigInfo | null>(null);

  // ── Credential input (first run) ─────────────────────────────────────
  const [credMode, setCredMode] = useState<CredMode>("inline");
  const [region, setRegion] = useState("us-east-1");
  const [systemName, setSystemName] = useState("claria");
  const [accessKeyId, setAccessKeyId] = useState("");
  const [secretAccessKey, setSecretAccessKey] = useState("");
  const [showSecret, setShowSecret] = useState(false);
  const [profileName, setProfileName] = useState("");
  const [profiles, setProfiles] = useState<string[]>([]);

  // Sub-account fields
  const [subAccountId, setSubAccountId] = useState("");
  const [roleName, setRoleName] = useState(DEFAULT_ROLE_NAME);
  const [assumeRoleResult, setAssumeRoleResult] = useState<AssumeRoleResult | null>(null);

  // ── Escalation (inline elevated creds) ───────────────────────────────
  const [escAccessKeyId, setEscAccessKeyId] = useState("");
  const [escSecretAccessKey, setEscSecretAccessKey] = useState("");
  const [showEscSecret, setShowEscSecret] = useState(false);

  // ── Reconciliation state ─────────────────────────────────────────────
  const [phase, setPhase] = useState<Phase>("loading");
  const [entries, setEntries] = useState<PlanEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { scanItems, applyItems, progressHandler, resetProgress } =
    useProvisionerProgress();
  const [resettingState, setResettingState] = useState(false);

  /**
   * Run one AWS phase transition: enter `during`, clear stale error and
   * progress, and land in "error" if `fn` throws. `fn` sets its own success
   * phase — different flows finish in "planned", "done", or "key_limit".
   */
  async function runPhase(during: Phase, fn: () => Promise<void>) {
    setPhase(during);
    setError(null);
    resetProgress();
    try {
      await fn();
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  }

  // ── Access-key limit recovery ────────────────────────────────────────
  const [keyLimit, setKeyLimit] = useState<AccessKeyLimitReached | null>(null);
  const [existingKeys, setExistingKeys] = useState<AccessKeyInfo[]>([]);
  const [loadingKeys, setLoadingKeys] = useState(false);
  const [keysError, setKeysError] = useState<string | null>(null);
  const [deletingKeyId, setDeletingKeyId] = useState<string | null>(null);

  // ── Management state ─────────────────────────────────────────────────
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [showDestroyConfirm, setShowDestroyConfirm] = useState(false);
  const [deleting, setDeleting] = useState(false);

  // Build credentials from form state.
  const buildCredentials = useCallback((): CredentialSource => {
    if (assumeRoleResult) {
      return {
        type: "inline",
        access_key_id: assumeRoleResult.access_key_id,
        secret_access_key: assumeRoleResult.secret_access_key,
        session_token: assumeRoleResult.session_token,
      };
    }
    switch (credMode) {
      case "inline":
        return { type: "inline", access_key_id: accessKeyId, secret_access_key: secretAccessKey, session_token: null };
      case "profile":
        return { type: "profile", profile_name: profileName };
      case "default_chain":
        return { type: "default_chain" };
      case "sub_account":
        return { type: "inline", access_key_id: accessKeyId, secret_access_key: secretAccessKey, session_token: null };
    }
  }, [credMode, accessKeyId, secretAccessKey, profileName, assumeRoleResult]);

  // ── Initialization ───────────────────────────────────────────────────
  useEffect(() => {
    (async () => {
      const exists = await hasConfig().catch(() => false);
      setConfigExists(exists);
      if (exists) {
        // Day-2: auto-scan using saved credentials.
        await runPhase("scanning", async () => {
          const info = await loadConfig();
          setConfig(info);
          const result = await plan(progressHandler);
          setEntries(result);
          setPhase("planned");
        });
      } else {
        setPhase("input");
        listAwsProfiles().then(setProfiles).catch(() => {});
      }
    })();
    // runPhase is a plain closure over stable setters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [progressHandler, resetProgress]);

  // ── First-run: scan with provided creds ──────────────────────────────
  async function handleInitialScan() {
    await runPhase("scanning", async () => {
      // If sub-account mode and not yet assumed role, do that first.
      if (credMode === "sub_account" && !assumeRoleResult) {
        const creds: CredentialSource = {
          type: "inline", access_key_id: accessKeyId, secret_access_key: secretAccessKey, session_token: null,
        };
        const result = await assumeRole(region, creds, subAccountId, roleName);
        setAssumeRoleResult(result);
        // Now scan with assumed-role creds.
        const assumedCreds: CredentialSource = {
          type: "inline",
          access_key_id: result.access_key_id,
          secret_access_key: result.secret_access_key,
          session_token: result.session_token,
        };
        const scanRes = await provisionScan(region, systemName, assumedCreds, progressHandler);
        setEntries(scanRes.entries);
        setPhase("planned");
        return;
      }

      const creds = buildCredentials();
      const scanRes = await provisionScan(region, systemName, creds, progressHandler);
      setEntries(scanRes.entries);
      setPhase("planned");
    });
  }

  // ── Apply changes ────────────────────────────────────────────────────
  async function handleApply() {
    if (configExists) {
      // Day-2 flow: use existing plan/apply commands.
      await runPhase("applying", async () => {
        const result = await apply(progressHandler);
        setEntries(result);
        setPhase("done");
      });
      return;
    }

    // First-run flow: use unified provision_apply.
    await runPhase("applying", async () => {
      const creds = buildCredentials();

      // Determine if we need to pass elevated credentials.
      // On first run with root/admin creds, these ARE the elevated creds.
      const assessment = await assessCredentials(region, creds);
      const isElevated =
        assessment.credential_class === "root" ||
        assessment.credential_class === "iam_admin";

      const outcome = await provisionApply(
        region,
        systemName,
        creds,
        isElevated ? creds : null,
        progressHandler,
      );

      // AWS resources may be untouched and this computer still unconfigured:
      // the handoff could not mint a key because both IAM slots are taken.
      if (outcome.access_key_limit) {
        setKeyLimit(outcome.access_key_limit);
        setPhase("key_limit");
        void loadExistingKeys(creds);
        return;
      }

      setEntries(outcome.entries);
      setConfigExists(true);
      setPhase("done");
    });
  }

  // ── Access-key limit recovery ────────────────────────────────────────

  async function loadExistingKeys(creds: CredentialSource) {
    setLoadingKeys(true);
    setKeysError(null);
    try {
      setExistingKeys(await listUserAccessKeys(region, creds));
    } catch (e) {
      setExistingKeys([]);
      setKeysError(String(e));
    } finally {
      setLoadingKeys(false);
    }
  }

  // Delete the key the operator explicitly confirmed, then retry the setup
  // now that a slot is free. Never called without that confirmation.
  async function handleDeleteKey(keyId: string) {
    setDeletingKeyId(keyId);
    setKeysError(null);
    try {
      await deleteUserAccessKey(region, buildCredentials(), keyId);
    } catch (e) {
      setKeysError(String(e));
      setDeletingKeyId(null);
      return;
    }
    setDeletingKeyId(null);
    setKeyLimit(null);
    setExistingKeys([]);
    void handleApply();
  }

  function cancelKeyLimit() {
    setKeyLimit(null);
    setExistingKeys([]);
    setKeysError(null);
    setPhase("planned");
  }

  // ── Escalation apply (day-2 with elevated creds) ─────────────────────
  async function handleEscalationApply() {
    await runPhase("applying", async () => {
      // Load the saved scoped creds from config for regular resources.
      const savedCreds = await loadConfig();
      // The escalation creds are from the inline form.
      const elevatedCreds: CredentialSource = {
        type: "inline",
        access_key_id: escAccessKeyId,
        secret_access_key: escSecretAccessKey,
        session_token: null,
      };

      // Use provision_apply for two-phase execution.
      const outcome = await provisionApply(
        savedCreds.region,
        savedCreds.system_name,
        // Regular creds — rebuild from config.
        // Note: we re-use the plan/apply path since config exists.
        elevatedCreds, // credentials param (we use elevated for scanning)
        elevatedCreds, // elevated_credentials param
        progressHandler,
      );
      // The handoff only runs when no local config exists, which is never
      // true here — this machine is already configured.
      setEntries(outcome.entries);
      setPhase("done");
    });
  }

  // ── Day-2 re-scan ────────────────────────────────────────────────────
  async function handleRescan() {
    await runPhase("scanning", async () => {
      const result = await plan(progressHandler);
      setEntries(result);
      setPhase("planned");
    });
  }

  // ── Destroy all resources ────────────────────────────────────────────
  async function handleDestroy() {
    setShowDestroyConfirm(false);
    await runPhase("applying", async () => {
      await destroy();
      await handleRescan();
    });
  }

  // ── Delete system ────────────────────────────────────────────────────
  async function handleDeleteSystem() {
    setShowDeleteConfirm(false);
    setDeleting(true);
    try {
      await deleteConfig();
      navigate("start");
    } catch (e) {
      setError(String(e));
      setDeleting(false);
    }
  }

  // ── Reset provisioner state ──────────────────────────────────────────
  async function handleResetState() {
    setResettingState(true);
    try {
      await resetProvisionerState();
      handleRescan();
    } catch (e) {
      setError(String(e));
    } finally {
      setResettingState(false);
    }
  }

  // ── Single-step back navigation ──────────────────────────────────────
  // Step back one phase in-component (preserving all form useState) instead
  // of unmounting to the start screen and losing everything the user typed.
  function phaseBack() {
    // Never interrupt an in-flight AWS call: post-await setState in
    // handleInitialScan/handleApply is unguarded and would yank the user
    // forward again. Back is also disabled in these phases.
    if (phase === "scanning" || phase === "applying") return;
    // Never let Back look like it resolved the key limit. Step back to the
    // plan with every key untouched, same as the panel's own Cancel.
    if (phase === "key_limit") {
      if (deletingKeyId !== null) return;
      cancelKeyLimit();
      return;
    }
    // First run: return to the credential form without unmounting.
    if (configExists === false && (phase === "planned" || phase === "error")) {
      setAssumeRoleResult(null);
      setError(null);
      setPhase("input");
      return;
    }
    // Mirror the escalation Cancel button so header Back is consistent.
    if (phase === "escalation") {
      setPhase("planned");
      return;
    }
    // input, done, and all day-2 phases: leave the screen.
    navigate("start");
  }

  // Check if plan needs escalation
  const needsEscalation = findEscalationEntry(entries) !== null;

  // Actions offered once a scan finishes.
  //
  // A conformant plan still needs an action on first run. Onboarding a second
  // computer against an already-provisioned account scans clean, but the
  // scoped access key and the local config are minted by `provision_apply`,
  // so "nothing to change in AWS" is not the same as "this computer is set
  // up". Without an action here the wizard is a dead end.
  function plannedActions() {
    if (hasChanges(entries)) {
      if (needsEscalation && !configExists) {
        return (
          <button
            onClick={handleApply}
            className="flex-1 py-2.5 bg-blue-500 text-white rounded-lg font-medium hover:bg-blue-600 transition-colors"
          >
            Bootstrap &amp; Apply
          </button>
        );
      }
      if (needsEscalation && configExists) {
        return (
          <button
            onClick={() => setPhase("escalation")}
            className="flex-1 py-2.5 bg-amber-500 text-white rounded-lg font-medium hover:bg-amber-600 transition-colors"
          >
            Provide Admin Credentials
          </button>
        );
      }
      return (
        <button
          onClick={handleApply}
          className="flex-1 py-2.5 bg-blue-500 text-white rounded-lg font-medium hover:bg-blue-600 transition-colors"
        >
          Apply Changes
        </button>
      );
    }

    if (!configExists) {
      return (
        <button
          onClick={handleApply}
          className="flex-1 py-2.5 bg-blue-500 text-white rounded-lg font-medium hover:bg-blue-600 transition-colors"
        >
          Set Up This Computer
        </button>
      );
    }

    return (
      <button
        onClick={() => navigate("start")}
        className="flex-1 py-2.5 bg-blue-500 text-white rounded-lg font-medium hover:bg-blue-600 transition-colors"
      >
        Continue to Claria
      </button>
    );
  }

  // ── Render ───────────────────────────────────────────────────────────

  if (phase === "loading") {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <p className="text-gray-500">Loading...</p>
      </div>
    );
  }

  return (
    <div className="min-h-screen p-6 max-w-2xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-xl font-bold">AWS Infrastructure</h2>
        <div className="flex items-center gap-3">
          {configExists && entries && (
            <button
              onClick={() => navigate("infra-chat")}
              className="text-sm text-blue-500 hover:text-blue-700"
            >
              Ask AI
            </button>
          )}
          <button
            onClick={phaseBack}
            disabled={phase === "scanning" || phase === "applying"}
            className="text-sm text-gray-500 hover:text-gray-700 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {configExists === false &&
            (phase === "planned" || phase === "error" || phase === "escalation")
              ? "Back to Credentials"
              : "Back"}
          </button>
        </div>
      </div>

      {/* Config info bar (day-2) */}
      {config && (
        <div className="bg-gray-50 rounded-lg p-3 mb-4 text-sm text-gray-600">
          <span className="font-medium">{config.system_name}</span>
          <span className="mx-2 text-gray-300">|</span>
          {config.region}
          <span className="mx-2 text-gray-300">|</span>
          {config.account_id}
        </div>
      )}

      {/* ── Phase: Credential input (first run) ────────────────────── */}
      {phase === "input" && (
        <div className="space-y-4">
          <p className="text-sm text-gray-600">
            Enter your AWS credentials to set up Claria. We'll create a
            least-privilege IAM user and provision all required resources.
          </p>

          {/* Region + system name */}
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-xs font-medium text-gray-500 mb-1">
                Region
              </label>
              <select
                value={region}
                onChange={(e) => setRegion(e.target.value)}
                className="w-full px-3 py-2 border rounded-lg text-sm"
              >
                {AWS_REGIONS.map((r) => (
                  <option key={r} value={r}>{r}</option>
                ))}
              </select>
            </div>
            <div>
              <label className="block text-xs font-medium text-gray-500 mb-1">
                System name
              </label>
              <input
                type="text"
                value={systemName}
                onChange={(e) => setSystemName(e.target.value)}
                className="w-full px-3 py-2 border rounded-lg text-sm"
                placeholder="claria"
              />
            </div>
          </div>

          {/* Credential mode tabs */}
          <div className="flex gap-1 p-1 bg-gray-100 rounded-lg">
            {([
              ["inline", "Access Key"],
              ["sub_account", "Sub-Account"],
              ["profile", "AWS Profile"],
              ["default_chain", "Default"],
            ] as [CredMode, string][]).map(([mode, label]) => (
              <button
                key={mode}
                onClick={() => { setCredMode(mode); setAssumeRoleResult(null); }}
                className={`flex-1 px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
                  credMode === mode
                    ? "bg-white shadow-sm text-gray-900"
                    : "text-gray-500 hover:text-gray-700"
                }`}
              >
                {label}
              </button>
            ))}
          </div>

          {/* Credential fields */}
          {(credMode === "inline" || credMode === "sub_account") && (
            <div className="space-y-3">
              <input
                type="text"
                value={accessKeyId}
                onChange={(e) => setAccessKeyId(e.target.value)}
                placeholder="Access Key ID"
                className="w-full px-3 py-2 border rounded-lg text-sm font-mono"
              />
              <div className="relative">
                <input
                  type={showSecret ? "text" : "password"}
                  value={secretAccessKey}
                  onChange={(e) => setSecretAccessKey(e.target.value)}
                  placeholder="Secret Access Key"
                  className="w-full px-3 py-2 border rounded-lg text-sm font-mono pr-16"
                />
                <button
                  type="button"
                  onClick={() => setShowSecret(!showSecret)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-xs text-gray-400 hover:text-gray-600"
                >
                  {showSecret ? "Hide" : "Show"}
                </button>
              </div>
            </div>
          )}

          {credMode === "sub_account" && (
            <div className="space-y-3">
              <input
                type="text"
                value={subAccountId}
                onChange={(e) => setSubAccountId(e.target.value)}
                placeholder="Sub-Account ID (12 digits)"
                className="w-full px-3 py-2 border rounded-lg text-sm font-mono"
              />
              <input
                type="text"
                value={roleName}
                onChange={(e) => setRoleName(e.target.value)}
                placeholder="Role name"
                className="w-full px-3 py-2 border rounded-lg text-sm font-mono"
              />
            </div>
          )}

          {credMode === "profile" && (
            <select
              value={profileName}
              onChange={(e) => setProfileName(e.target.value)}
              className="w-full px-3 py-2 border rounded-lg text-sm"
            >
              <option value="">Select a profile...</option>
              {profiles.map((p) => (
                <option key={p} value={p}>{p}</option>
              ))}
            </select>
          )}

          {credMode === "default_chain" && (
            <p className="text-xs text-gray-500">
              Uses the default AWS credential chain (environment variables,
              config files, instance profile, etc.)
            </p>
          )}

          <button
            onClick={handleInitialScan}
            disabled={
              (credMode === "inline" && (!accessKeyId || !secretAccessKey)) ||
              (credMode === "sub_account" && (!accessKeyId || !secretAccessKey || !subAccountId)) ||
              (credMode === "profile" && !profileName)
            }
            className="w-full py-2.5 bg-blue-500 text-white rounded-lg font-medium hover:bg-blue-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Scan Resources
          </button>
        </div>
      )}

      {/* ── Lifecycle: scanning → planned → applying → done / error ── */}
      {(phase === "scanning" ||
        phase === "planned" ||
        phase === "applying" ||
        phase === "done" ||
        phase === "error") && (
        <InfraState
          phase={phase}
          entries={entries}
          scanItems={scanItems}
          applyItems={applyItems}
          error={error}
          showEscalationNotice={needsEscalation && configExists === true}
          actions={
            phase === "planned" ? (
              <div className="space-y-3">
                {!hasChanges(entries) && !configExists && (
                  <p className="text-sm text-gray-600">
                    This AWS account is already set up. Claria will create an
                    access key for this computer and save its configuration
                    locally — no AWS resources will change.
                  </p>
                )}
                <div className="flex gap-2">{plannedActions()}</div>
              </div>
            ) : phase === "done" ? (
              <div className="flex gap-2">
                <button
                  onClick={handleRescan}
                  className="px-4 py-2 border rounded-lg text-sm text-gray-600 hover:bg-gray-50"
                >
                  Re-scan
                </button>
                <button
                  onClick={() => navigate("start")}
                  className="flex-1 py-2.5 bg-blue-500 text-white rounded-lg font-medium hover:bg-blue-600 transition-colors"
                >
                  Continue to Claria
                </button>
              </div>
            ) : undefined
          }
          errorActions={
            <div className="flex gap-2">
              {configExists ? (
                <button
                  onClick={handleRescan}
                  className="flex-1 py-2 bg-blue-500 text-white rounded-lg text-sm hover:bg-blue-600"
                >
                  Retry Scan
                </button>
              ) : (
                <button
                  onClick={() => { setPhase("input"); setError(null); }}
                  className="flex-1 py-2 bg-blue-500 text-white rounded-lg text-sm hover:bg-blue-600"
                >
                  Back to Credentials
                </button>
              )}
              <button
                onClick={handleResetState}
                disabled={resettingState}
                className="px-4 py-2 border rounded-lg text-sm text-gray-600 hover:bg-gray-50 disabled:opacity-50"
              >
                {resettingState ? "Resetting..." : "Reset State"}
              </button>
            </div>
          }
        />
      )}

      {/* ── Phase: Access-key limit recovery ───────────────────────── */}
      {phase === "key_limit" && keyLimit && (
        <AccessKeyLimitPanel
          limit={keyLimit}
          keys={existingKeys}
          loadingKeys={loadingKeys}
          keysError={keysError}
          deletingKeyId={deletingKeyId}
          onDelete={handleDeleteKey}
          onCancel={cancelKeyLimit}
        />
      )}

      {/* ── Phase: Escalation ──────────────────────────────────────── */}
      {phase === "escalation" && (
        <div className="space-y-4">
          <div className="bg-amber-50 border border-amber-200 rounded-lg p-4">
            <p className="text-sm font-medium text-amber-800 mb-1">
              Admin credentials required
            </p>
            <p className="text-xs text-amber-700">
              The IAM policy needs to be updated. Enter root or IAM admin
              credentials — they'll be used once and discarded.
            </p>
          </div>

          <input
            type="text"
            value={escAccessKeyId}
            onChange={(e) => setEscAccessKeyId(e.target.value)}
            placeholder="Admin Access Key ID"
            className="w-full px-3 py-2 border rounded-lg text-sm font-mono"
          />
          <div className="relative">
            <input
              type={showEscSecret ? "text" : "password"}
              value={escSecretAccessKey}
              onChange={(e) => setEscSecretAccessKey(e.target.value)}
              placeholder="Admin Secret Access Key"
              className="w-full px-3 py-2 border rounded-lg text-sm font-mono pr-16"
            />
            <button
              type="button"
              onClick={() => setShowEscSecret(!showEscSecret)}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-xs text-gray-400 hover:text-gray-600"
            >
              {showEscSecret ? "Hide" : "Show"}
            </button>
          </div>

          <div className="flex gap-2">
            <button
              onClick={() => setPhase("planned")}
              className="px-4 py-2 border rounded-lg text-sm text-gray-600 hover:bg-gray-50"
            >
              Cancel
            </button>
            <button
              onClick={handleEscalationApply}
              disabled={!escAccessKeyId || !escSecretAccessKey}
              className="flex-1 py-2.5 bg-amber-500 text-white rounded-lg font-medium hover:bg-amber-600 transition-colors disabled:opacity-50"
            >
              Apply with Elevated Credentials
            </button>
          </div>
        </div>
      )}

      {/* ── Management actions (day-2) ──────────────────────────────── */}
      {configExists && (phase === "planned" || phase === "done") && (
        <div className="mt-8 pt-6 border-t border-gray-200">
          <details>
            <summary className="text-xs font-semibold text-gray-400 uppercase tracking-wide cursor-pointer hover:text-gray-600">
              Advanced
            </summary>
            <div className="mt-3 space-y-2">
              <button
                onClick={handleResetState}
                disabled={resettingState}
                className="w-full py-2 border rounded-lg text-sm text-gray-600 hover:bg-gray-50 disabled:opacity-50"
              >
                {resettingState ? "Resetting..." : "Reset Provisioner State"}
              </button>

              <button
                onClick={() => setShowDestroyConfirm(true)}
                className="w-full py-2 border border-red-200 rounded-lg text-sm text-red-600 hover:bg-red-50"
              >
                Destroy All Resources
              </button>

              <button
                onClick={() => setShowDeleteConfirm(true)}
                className="w-full py-2 border border-red-200 rounded-lg text-sm text-red-600 hover:bg-red-50"
              >
                Delete System Configuration
              </button>
            </div>
          </details>

          {/* Destroy confirm dialog */}
          {showDestroyConfirm && (
            <div className="mt-3 bg-red-50 border border-red-200 rounded-lg p-4">
              <p className="text-sm text-red-800 mb-3">
                This will delete all AWS resources (S3 bucket, CloudTrail, etc.).
                Your data will be permanently lost.
              </p>
              <div className="flex gap-2">
                <button
                  onClick={() => setShowDestroyConfirm(false)}
                  className="flex-1 py-2 border rounded-lg text-sm"
                >
                  Cancel
                </button>
                <button
                  onClick={handleDestroy}
                  className="flex-1 py-2 bg-red-600 text-white rounded-lg text-sm hover:bg-red-700"
                >
                  Destroy
                </button>
              </div>
            </div>
          )}

          {/* Delete config confirm dialog */}
          {showDeleteConfirm && (
            <div className="mt-3 bg-red-50 border border-red-200 rounded-lg p-4">
              <p className="text-sm text-red-800 mb-3">
                This will delete the local configuration. AWS resources will
                remain intact.
              </p>
              <div className="flex gap-2">
                <button
                  onClick={() => setShowDeleteConfirm(false)}
                  className="flex-1 py-2 border rounded-lg text-sm"
                >
                  Cancel
                </button>
                <button
                  onClick={handleDeleteSystem}
                  disabled={deleting}
                  className="flex-1 py-2 bg-red-600 text-white rounded-lg text-sm hover:bg-red-700 disabled:opacity-50"
                >
                  {deleting ? "Deleting..." : "Delete Config"}
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
