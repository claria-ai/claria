import { useState, useEffect } from "react";
import StepIndicator from "../components/StepIndicator";
import {
  saveConfig,
  assessCredentials,
  assumeRole,
  bootstrapIamUser,
  listAwsProfiles,
  listUserAccessKeys,
  deleteUserAccessKey,
  type CredentialSource,
  type CredentialAssessment,
  type AssumeRoleResult,
  type BootstrapResult,
  type BootstrapStep,
  type AccessKeyInfo,
} from "../lib/tauri";
import type { Page } from "../App";

const AWS_REGIONS = [
  "us-east-1",
  "us-east-2",
  "us-west-1",
  "us-west-2",
  "eu-west-1",
  "eu-west-2",
  "eu-west-3",
  "eu-central-1",
  "eu-north-1",
  "ap-southeast-1",
  "ap-southeast-2",
  "ap-northeast-1",
  "ap-northeast-2",
  "ap-south-1",
  "ca-central-1",
  "sa-east-1",
];

const DEFAULT_ROLE_NAME = "OrganizationAccountAccessRole";

type Mode = "inline" | "sub_account" | "profile" | "default_chain";

type Phase =
  | "input"            // Entering credentials
  | "assuming_role"    // Calling STS AssumeRole (sub-account only)
  | "role_assumed"     // Role assumed, showing sub-account identity
  | "assessing"        // Probing credentials
  | "assessed"         // Assessment complete, showing classification
  | "bootstrapping"    // Running bootstrap flow
  | "key_limit"        // 2-key limit hit — operator picks a key to delete
  | "bootstrap_done"   // Bootstrap finished (success or failure)
  | "saving"           // Writing config for scoped creds
  | "done";            // All done, ready to advance

export default function CredentialIntake({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  const [mode, setMode] = useState<Mode>("inline");
  const [region, setRegion] = useState("us-east-1");
  const [systemName, setSystemName] = useState("claria");
  const [accessKeyId, setAccessKeyId] = useState("");
  const [secretAccessKey, setSecretAccessKey] = useState("");
  const [showSecret, setShowSecret] = useState(false);
  const [profileName, setProfileName] = useState("");
  const [profiles, setProfiles] = useState<string[]>([]);

  // Sub-account specific fields
  const [subAccountId, setSubAccountId] = useState("");
  const [roleName, setRoleName] = useState(DEFAULT_ROLE_NAME);
  const [assumeRoleResult, setAssumeRoleResult] = useState<AssumeRoleResult | null>(null);

  const [phase, setPhase] = useState<Phase>("input");
  const [assessment, setAssessment] = useState<CredentialAssessment | null>(null);
  const [bootstrapResult, setBootstrapResult] = useState<BootstrapResult | null>(null);
  const [existingKeys, setExistingKeys] = useState<AccessKeyInfo[]>([]);
  const [deletingKeyId, setDeletingKeyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listAwsProfiles()
      .then(setProfiles)
      .catch(() => setProfiles([]));
  }, []);

  const systemNameValid = /^[a-z0-9][a-z0-9-]{1,38}[a-z0-9]$/.test(systemName);
  const subAccountIdValid = /^\d{12}$/.test(subAccountId);
  const roleNameValid = roleName.length > 0;

  function buildCredentials(): CredentialSource {
    switch (mode) {
      case "inline":
      case "sub_account":
        return {
          type: "inline",
          access_key_id: accessKeyId,
          secret_access_key: secretAccessKey,
        };
      case "profile":
        return { type: "profile", profile_name: profileName };
      case "default_chain":
        return { type: "default_chain" };
    }
  }

  /**
   * Build a CredentialSource from the assumed-role temporary credentials.
   * Includes session_token for STS temporary credential support.
   */
  function buildAssumedRoleCredentials(): CredentialSource | null {
    if (!assumeRoleResult) return null;
    return {
      type: "inline",
      access_key_id: assumeRoleResult.access_key_id,
      secret_access_key: assumeRoleResult.secret_access_key,
      session_token: assumeRoleResult.session_token,
    };
  }

  function resetAll() {
    setPhase("input");
    setAssessment(null);
    setBootstrapResult(null);
    setAssumeRoleResult(null);
    setExistingKeys([]);
    setDeletingKeyId(null);
    setError(null);
  }

  // ── Sub-account: Assume Role ────────────────────────────────────────────

  async function handleAssumeRole() {
    setPhase("assuming_role");
    setError(null);
    setAssumeRoleResult(null);
    setAssessment(null);
    setBootstrapResult(null);
    try {
      const result = await assumeRole(
        region,
        buildCredentials(),
        subAccountId,
        roleName
      );
      setAssumeRoleResult(result);
      setPhase("role_assumed");
    } catch (e) {
      setError(String(e));
      setPhase("input");
    }
  }

  // ── Assess credentials ─────────────────────────────────────────────────

  async function handleAssess() {
    setPhase("assessing");
    setError(null);
    setAssessment(null);
    setBootstrapResult(null);
    try {
      // For sub-account mode, assess the assumed-role temp credentials
      const creds =
        mode === "sub_account"
          ? buildAssumedRoleCredentials()
          : buildCredentials();

      if (!creds) {
        setError("No credentials available. Please assume the role first.");
        setPhase("input");
        return;
      }

      const result = await assessCredentials(region, creds);
      setAssessment(result);
      setPhase("assessed");
    } catch (e) {
      setError(String(e));
      setPhase(mode === "sub_account" && assumeRoleResult ? "role_assumed" : "input");
    }
  }

  // ── Bootstrap ──────────────────────────────────────────────────────────

  async function handleBootstrap() {
    if (!assessment) return;
    setPhase("bootstrapping");
    setError(null);
    try {
      // Determine which credentials to bootstrap with
      let keyId: string;
      let secret: string;
      let sessionToken: string | null = null;

      if (mode === "sub_account" && assumeRoleResult) {
        // Use the assumed-role temporary credentials
        keyId = assumeRoleResult.access_key_id;
        secret = assumeRoleResult.secret_access_key;
        sessionToken = assumeRoleResult.session_token;
      } else {
        // Use the directly-provided credentials
        keyId = accessKeyId;
        secret = secretAccessKey;
      }

      const result = await bootstrapIamUser(
        region,
        systemName,
        keyId,
        secret,
        sessionToken,
        assessment.credential_class
      );
      setBootstrapResult(result);

      // Check if the failure was due to the 2-key limit on the IAM user.
      const keyStep = result.steps.find((s) => s.name === "create_access_key");
      if (!result.success && keyStep?.status === "failed" && keyStep.detail === "key_limit_exceeded") {
        // Fetch existing keys so the operator can pick one to delete.
        const creds =
          mode === "sub_account"
            ? buildAssumedRoleCredentials() ?? buildCredentials()
            : buildCredentials();
        try {
          const keys = await listUserAccessKeys(region, creds);
          setExistingKeys(keys);
        } catch {
          setExistingKeys([]);
        }
        setPhase("key_limit");
      } else {
        setPhase("bootstrap_done");
      }
    } catch (e) {
      setError(String(e));
      setPhase("assessed");
    }
  }

  // ── Save scoped credentials directly ───────────────────────────────────

  async function handleSaveScoped() {
    if (!assessment) return;
    setPhase("saving");
    setError(null);
    try {
      // For sub-account mode with scoped credentials, save the temp creds
      // Note: this shouldn't normally happen — sub-account flow goes through
      // bootstrap. But handle it gracefully.
      const creds =
        mode === "sub_account"
          ? buildAssumedRoleCredentials() ?? buildCredentials()
          : buildCredentials();
      await saveConfig(region, systemName, creds);
      setPhase("done");
    } catch (e) {
      setError(String(e));
      setPhase("assessed");
    }
  }

  // ── Delete an access key and retry bootstrap ────────────────────────────

  async function handleDeleteKeyAndRetry(keyIdToDelete: string) {
    if (!assessment) return;
    setDeletingKeyId(keyIdToDelete);
    setError(null);
    try {
      const creds =
        mode === "sub_account"
          ? buildAssumedRoleCredentials() ?? buildCredentials()
          : buildCredentials();
      await deleteUserAccessKey(region, creds, keyIdToDelete);
      // Clear the key-limit UI and the old failed bootstrap result so
      // the "Setting up your secure IAM user..." spinner is visible
      // while the retry runs.
      setExistingKeys([]);
      setDeletingKeyId(null);
      setBootstrapResult(null);
      // Re-attempt bootstrap now that there's room for a new key.
      handleBootstrap();
    } catch (e) {
      setError(String(e));
      setDeletingKeyId(null);
    }
  }

  function handleContinue() {
    navigate("scan");
  }

  // ── Computed state ─────────────────────────────────────────────────────

  const canAssumeRole =
    mode === "sub_account" &&
    systemNameValid &&
    subAccountIdValid &&
    roleNameValid &&
    accessKeyId.length > 0 &&
    secretAccessKey.length > 0;

  const canAssess =
    systemNameValid &&
    (mode === "sub_account"
      ? !!assumeRoleResult // Must have assumed role first
      : mode === "inline"
        ? accessKeyId.length > 0 && secretAccessKey.length > 0
        : mode === "profile"
          ? profileName.length > 0
          : true);

  const isWorking =
    phase === "assuming_role" ||
    phase === "assessing" ||
    phase === "bootstrapping" ||
    phase === "saving" ||
    deletingKeyId !== null;

  // Fields are locked after role assumption or assessment
  const fieldsDisabled =
    isWorking ||
    (mode === "sub_account" && phase !== "input") ||
    (mode !== "sub_account" && phase !== "input" && phase !== "assessed");

  return (
    <div className="max-w-2xl mx-auto p-8">
      <StepIndicator current={3} />

      <h2 className="text-2xl font-bold mb-6">Step 3: Configure Credentials</h2>

      {/* Mode selector */}
      <div className="flex flex-wrap gap-2 mb-6">
        {([
          { value: "inline" as Mode, label: "I'm new to AWS" },
          { value: "sub_account" as Mode, label: "Sub-account" },
          { value: "profile" as Mode, label: "Existing profile" },
          { value: "default_chain" as Mode, label: "Default credentials" },
        ]).map(({ value, label }) => (
          <button
            key={value}
            onClick={() => {
              setMode(value);
              resetAll();
            }}
            disabled={isWorking}
            className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
              mode === value
                ? "bg-blue-500 text-white"
                : "bg-white text-gray-700 border border-gray-300 hover:bg-gray-50"
            } disabled:opacity-50`}
          >
            {label}
          </button>
        ))}
      </div>

      {/* Sub-account explainer */}
      {mode === "sub_account" && phase === "input" && (
        <div className="bg-indigo-50 border border-indigo-200 rounded-lg p-4 mb-6">
          <p className="text-indigo-900 text-sm font-medium mb-1">
            AWS Organizations Sub-Account
          </p>
          <p className="text-indigo-800 text-sm">
            Enter your <strong>parent account</strong> credentials and the sub-account ID.
            Claria will assume the{" "}
            <code className="text-xs bg-indigo-100 px-1 rounded">
              OrganizationAccountAccessRole
            </code>{" "}
            in the sub-account, then create a dedicated IAM user with minimal permissions.
          </p>
        </div>
      )}

      <div className="space-y-4">
        {/* Region */}
        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1">
            AWS Region
          </label>
          <select
            value={region}
            onChange={(e) => { setRegion(e.target.value); resetAll(); }}
            disabled={fieldsDisabled}
            className="w-full px-3 py-2 border border-gray-300 rounded-lg bg-white disabled:opacity-50"
          >
            {AWS_REGIONS.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
        </div>

        {/* System Name */}
        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1">
            System Name
          </label>
          <input
            type="text"
            value={systemName}
            onChange={(e) => { setSystemName(e.target.value.toLowerCase()); resetAll(); }}
            placeholder="claria"
            disabled={fieldsDisabled}
            className={`w-full px-3 py-2 border rounded-lg disabled:opacity-50 ${
              systemName.length > 0 && !systemNameValid
                ? "border-red-300"
                : "border-gray-300"
            }`}
          />
          <p className="text-xs text-gray-500 mt-1">
            Lowercase letters, numbers, and hyphens. 3–40 characters.
          </p>
        </div>

        {/* ── Inline + Sub-account: Access key fields ───────────────────── */}
        {(mode === "inline" || mode === "sub_account") && (
          <>
            {mode === "sub_account" && (
              <div className="border-t border-gray-200 pt-4 mt-2">
                <p className="text-xs font-medium text-gray-500 uppercase tracking-wide mb-3">
                  Parent Account Credentials
                </p>
              </div>
            )}
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Access Key ID
              </label>
              <input
                type="text"
                value={accessKeyId}
                onChange={(e) => { setAccessKeyId(e.target.value); resetAll(); }}
                placeholder="AKIAIOSFODNN7EXAMPLE"
                disabled={fieldsDisabled}
                className="w-full px-3 py-2 border border-gray-300 rounded-lg font-mono text-sm disabled:opacity-50"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Secret Access Key
              </label>
              <div className="relative">
                <input
                  type={showSecret ? "text" : "password"}
                  value={secretAccessKey}
                  onChange={(e) => { setSecretAccessKey(e.target.value); resetAll(); }}
                  placeholder="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
                  disabled={fieldsDisabled}
                  className="w-full px-3 py-2 border border-gray-300 rounded-lg font-mono text-sm pr-16 disabled:opacity-50"
                />
                <button
                  type="button"
                  onClick={() => setShowSecret(!showSecret)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-xs text-gray-500 hover:text-gray-700"
                >
                  {showSecret ? "Hide" : "Show"}
                </button>
              </div>
            </div>
          </>
        )}

        {/* ── Sub-account: Account ID + Role Name ───────────────────────── */}
        {mode === "sub_account" && (
          <>
            <div className="border-t border-gray-200 pt-4 mt-2">
              <p className="text-xs font-medium text-gray-500 uppercase tracking-wide mb-3">
                Sub-Account Details
              </p>
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Sub-Account ID
              </label>
              <input
                type="text"
                value={subAccountId}
                onChange={(e) => {
                  // Allow only digits, max 12
                  const v = e.target.value.replace(/\D/g, "").slice(0, 12);
                  setSubAccountId(v);
                  resetAll();
                }}
                placeholder="690641653532"
                disabled={fieldsDisabled}
                className={`w-full px-3 py-2 border rounded-lg font-mono text-sm disabled:opacity-50 ${
                  subAccountId.length > 0 && !subAccountIdValid
                    ? "border-red-300"
                    : "border-gray-300"
                }`}
              />
              <p className="text-xs text-gray-500 mt-1">
                12-digit AWS account ID for the sub-account
              </p>
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Role Name
              </label>
              <input
                type="text"
                value={roleName}
                onChange={(e) => { setRoleName(e.target.value); resetAll(); }}
                placeholder={DEFAULT_ROLE_NAME}
                disabled={fieldsDisabled}
                className="w-full px-3 py-2 border border-gray-300 rounded-lg font-mono text-sm disabled:opacity-50"
              />
              <p className="text-xs text-gray-500 mt-1">
                Usually{" "}
                <code className="bg-gray-100 px-1 rounded text-xs">
                  {DEFAULT_ROLE_NAME}
                </code>{" "}
                for sub-accounts created via AWS Organizations
              </p>
            </div>
          </>
        )}

        {/* ── Profile mode fields ───────────────────────────────────────── */}
        {mode === "profile" && (
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">
              Profile Name
            </label>
            {profiles.length > 0 ? (
              <select
                value={profileName}
                onChange={(e) => { setProfileName(e.target.value); resetAll(); }}
                disabled={fieldsDisabled}
                className="w-full px-3 py-2 border border-gray-300 rounded-lg bg-white disabled:opacity-50"
              >
                <option value="">Select a profile...</option>
                {profiles.map((p) => (
                  <option key={p} value={p}>
                    {p}
                  </option>
                ))}
              </select>
            ) : (
              <input
                type="text"
                value={profileName}
                onChange={(e) => { setProfileName(e.target.value); resetAll(); }}
                placeholder="claria-admin"
                disabled={fieldsDisabled}
                className="w-full px-3 py-2 border border-gray-300 rounded-lg disabled:opacity-50"
              />
            )}
          </div>
        )}

        {/* ── Default chain info ────────────────────────────────────────── */}
        {mode === "default_chain" && (
          <div className="bg-gray-50 border border-gray-200 rounded-lg p-4">
            <p className="text-sm text-gray-600">
              Claria will use the standard AWS credential chain — environment
              variables, IAM roles, SSO sessions, or whatever your system
              provides.
            </p>
          </div>
        )}
      </div>

      {/* ── Role assumed success card (sub-account only) ──────────────── */}
      {mode === "sub_account" && assumeRoleResult && phase !== "input" && (
        <AssumedRoleCard result={assumeRoleResult} />
      )}

      {/* ── Assessment result ─────────────────────────────────────────── */}
      {assessment && <AssessmentCard assessment={assessment} />}

      {/* ── Root / Admin bootstrap notice ─────────────────────────────── */}
      {phase === "assessed" &&
        assessment &&
        (assessment.credential_class === "root" ||
          assessment.credential_class === "iam_admin") && (
          <BootstrapNotice
            credentialClass={assessment.credential_class}
            isSubAccount={mode === "sub_account"}
            onBootstrap={handleBootstrap}
          />
        )}

      {/* ── Scoped — ready to save ────────────────────────────────────── */}
      {phase === "assessed" &&
        assessment &&
        assessment.credential_class === "scoped_claria" && (
          <div className="bg-green-50 border border-green-200 rounded-lg p-4 mt-4">
            <p className="text-green-800 text-sm font-medium">
              ✅ Your credentials are already scoped for Claria. Click "Save &amp;
              Continue" to proceed.
            </p>
          </div>
        )}

      {/* ── Insufficient ──────────────────────────────────────────────── */}
      {phase === "assessed" &&
        assessment &&
        assessment.credential_class === "insufficient" && (
          <div className="bg-red-50 border border-red-200 rounded-lg p-4 mt-4">
            <p className="text-red-800 text-sm font-medium mb-1">
              ❌ Insufficient permissions
            </p>
            <p className="text-red-700 text-sm">{assessment.reason}</p>
          </div>
        )}

      {/* ── Bootstrap progress / result ───────────────────────────────── */}
      {(phase === "bootstrapping" || phase === "bootstrap_done" || phase === "key_limit") &&
        bootstrapResult && (
          <BootstrapProgress result={bootstrapResult} />
        )}

      {phase === "bootstrapping" && !bootstrapResult && (
        <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 mt-4">
          <p className="text-blue-800 text-sm flex items-center gap-2">
            <Spinner /> Setting up your secure IAM user...
          </p>
        </div>
      )}

      {/* ── Bootstrap success message ─────────────────────────────────── */}
      {phase === "bootstrap_done" && bootstrapResult?.success && (
        <div className="bg-green-50 border border-green-200 rounded-lg p-4 mt-4">
          <p className="text-green-800 text-sm font-medium">
            ✅ Secure IAM user created
            {mode === "sub_account" && assumeRoleResult
              ? ` in sub-account ${assumeRoleResult.account_id}`
              : ""}
            . Your configuration has been saved with scoped credentials.
            {assessment?.credential_class === "root" &&
              " The root access key has been deleted from AWS."}
          </p>
        </div>
      )}

      {/* ── Bootstrap failure message (not shown when key-limit is active) */}
      {phase === "bootstrap_done" && bootstrapResult && !bootstrapResult.success && (
        <div className="bg-red-50 border border-red-200 rounded-lg p-4 mt-4">
          <p className="text-red-800 text-sm font-medium mb-1">
            ❌ Bootstrap failed
          </p>
          <p className="text-red-700 text-sm">
            {bootstrapResult.error}
          </p>
          <p className="text-red-600 text-xs mt-2">
            Review the steps above. You may need to clean up partially-created
            resources in the IAM console.
          </p>
        </div>
      )}

      {/* ── Key limit — operator picks a key to delete ───────────────── */}
      {phase === "key_limit" && (
        <KeyLimitCard
          keys={existingKeys}
          deletingKeyId={deletingKeyId}
          onDelete={handleDeleteKeyAndRetry}
          onCancel={resetAll}
        />
      )}

      {/* ── General errors ────────────────────────────────────────────── */}
      {error && (
        <div className="bg-red-50 border border-red-200 rounded-lg p-4 mt-4">
          <p className="text-red-800 text-sm">{error}</p>
        </div>
      )}

      {/* ── Actions ───────────────────────────────────────────────────── */}
      <div className="flex justify-between mt-8">
        <button
          onClick={() => navigate("guide-iam")}
          disabled={isWorking}
          className="px-4 py-2 text-gray-600 hover:text-gray-800 disabled:opacity-50"
        >
          Back
        </button>
        <div className="flex gap-3">

          {/* Sub-account: Assume Role button (before role is assumed) */}
          {mode === "sub_account" && phase === "input" && (
            <button
              onClick={handleAssumeRole}
              disabled={!canAssumeRole || isWorking}
              className="px-6 py-2 bg-indigo-500 text-white rounded-lg hover:bg-indigo-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              Assume Role
            </button>
          )}

          {/* Sub-account: Assuming Role spinner */}
          {mode === "sub_account" && phase === "assuming_role" && (
            <button
              disabled
              className="px-6 py-2 bg-indigo-400 text-white rounded-lg opacity-50"
            >
              <span className="flex items-center gap-2">
                <Spinner /> Assuming role...
              </span>
            </button>
          )}

          {/* Sub-account: After role assumed, assess credentials */}
          {mode === "sub_account" && phase === "role_assumed" && (
            <button
              onClick={handleAssess}
              className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
            >
              Assess Sub-Account Credentials
            </button>
          )}

          {/* Non-sub-account: Assess button — visible in input phase */}
          {mode !== "sub_account" &&
            (phase === "input" || phase === "assessing") && (
              <button
                onClick={handleAssess}
                disabled={!canAssess || isWorking}
                className="px-6 py-2 bg-white text-gray-700 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {phase === "assessing" ? (
                  <span className="flex items-center gap-2">
                    <Spinner /> Checking...
                  </span>
                ) : (
                  "Check Credentials"
                )}
              </button>
            )}

          {/* Assessing spinner (sub-account) */}
          {mode === "sub_account" && phase === "assessing" && (
            <button
              disabled
              className="px-6 py-2 bg-blue-400 text-white rounded-lg opacity-50"
            >
              <span className="flex items-center gap-2">
                <Spinner /> Assessing...
              </span>
            </button>
          )}

          {/* Save button — visible when scoped */}
          {phase === "assessed" &&
            assessment?.credential_class === "scoped_claria" && (
              <button
                onClick={handleSaveScoped}
                className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
              >
                Save &amp; Continue
              </button>
            )}

          {/* Re-check button — visible after insufficient */}
          {phase === "assessed" &&
            assessment?.credential_class === "insufficient" && (
              <button
                onClick={resetAll}
                className="px-6 py-2 bg-white text-gray-700 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors"
              >
                Try Different Credentials
              </button>
            )}

          {/* Continue button — visible after successful bootstrap */}
          {phase === "bootstrap_done" && bootstrapResult?.success && (
            <button
              onClick={handleContinue}
              className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
            >
              Continue to Provisioning
            </button>
          )}

          {/* Retry button — visible after failed bootstrap */}
          {phase === "bootstrap_done" && bootstrapResult && !bootstrapResult.success && (
            <button
              onClick={resetAll}
              className="px-6 py-2 bg-white text-gray-700 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors"
            >
              Start Over
            </button>
          )}

          {/* Saving indicator */}
          {phase === "saving" && (
            <button
              disabled
              className="px-6 py-2 bg-blue-400 text-white rounded-lg opacity-50"
            >
              <span className="flex items-center gap-2">
                <Spinner /> Saving...
              </span>
            </button>
          )}

          {/* Done — waiting to advance */}
          {phase === "done" && (
            <button
              onClick={handleContinue}
              className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
            >
              Continue to Provisioning
            </button>
          )}
        </div>
      </div>
    </div>
  );
}


// ── Sub-components ──────────────────────────────────────────────────────────

function AssumedRoleCard({ result }: { result: AssumeRoleResult }) {
  return (
    <div className="border border-indigo-200 bg-indigo-50 rounded-lg p-4 mt-6">
      <div className="flex items-start gap-3">
        <span className="text-lg">🔗</span>
        <div className="flex-1 min-w-0">
          <p className="text-sm font-semibold text-indigo-900">
            Role Assumed Successfully
          </p>
          <p className="text-xs text-indigo-800 mt-1">
            You are now operating in the sub-account. Temporary credentials
            will be used to set up a dedicated IAM user.
          </p>
          <div className="mt-2 space-y-0.5">
            <p className="text-xs font-mono text-indigo-700">
              Account: {result.account_id}
            </p>
            <p className="text-xs font-mono text-indigo-700 truncate">
              Role: {result.assumed_role_arn}
            </p>
            {result.expiration && (
              <p className="text-xs font-mono text-indigo-600">
                Expires: {result.expiration}
              </p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function AssessmentCard({
  assessment,
}: {
  assessment: CredentialAssessment;
}) {
  const classLabels: Record<string, { label: string; color: string; icon: string }> = {
    root: {
      label: "Root Account",
      color: "text-amber-800 bg-amber-50 border-amber-200",
      icon: "⚠️",
    },
    iam_admin: {
      label: "IAM Admin",
      color: "text-amber-800 bg-amber-50 border-amber-200",
      icon: "🔑",
    },
    scoped_claria: {
      label: "Scoped Claria User",
      color: "text-green-800 bg-green-50 border-green-200",
      icon: "✅",
    },
    insufficient: {
      label: "Insufficient Permissions",
      color: "text-red-800 bg-red-50 border-red-200",
      icon: "❌",
    },
  };

  const cls = classLabels[assessment.credential_class] ?? {
    label: assessment.credential_class,
    color: "text-gray-800 bg-gray-50 border-gray-200",
    icon: "❓",
  };

  return (
    <div className={`border rounded-lg p-4 mt-6 ${cls.color}`}>
      <div className="flex items-start gap-3">
        <span className="text-lg">{cls.icon}</span>
        <div className="flex-1 min-w-0">
          <p className="text-sm font-semibold">{cls.label}</p>
          <p className="text-xs mt-1 opacity-80">{assessment.reason}</p>
          <div className="mt-2 space-y-0.5">
            <p className="text-xs font-mono opacity-70">
              Account: {assessment.identity.account_id}
            </p>
            <p className="text-xs font-mono opacity-70 truncate">
              ARN: {assessment.identity.arn}
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}

function BootstrapNotice({
  credentialClass,
  isSubAccount,
  onBootstrap,
}: {
  credentialClass: "root" | "iam_admin";
  isSubAccount: boolean;
  onBootstrap: () => void;
}) {
  const isRoot = credentialClass === "root";

  return (
    <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 mt-4">
      <p className="text-blue-900 text-sm font-medium mb-2">
        {isRoot
          ? "Root credentials detected"
          : isSubAccount
            ? "Admin role assumed in sub-account"
            : "Admin credentials detected"}
      </p>
      <p className="text-blue-800 text-sm mb-1">
        Claria will create a dedicated IAM user (<code className="text-xs bg-blue-100 px-1 rounded">claria-admin</code>) with
        minimal permissions scoped to only what Claria needs.
      </p>
      {isSubAccount && (
        <p className="text-blue-800 text-sm mb-1">
          The IAM user will be created <strong>in the sub-account</strong>. Your
          parent-account credentials and the assumed role are not stored.
        </p>
      )}
      {isRoot && !isSubAccount && (
        <p className="text-blue-800 text-sm mb-1">
          The root access key will be <strong>deleted from AWS</strong> after the
          new user is created. Root credentials will never be saved to disk.
        </p>
      )}
      {!isRoot && !isSubAccount && (
        <p className="text-blue-800 text-sm mb-1">
          Your current admin credentials will not be modified. Claria will use
          the new scoped user going forward.
        </p>
      )}
      <button
        onClick={onBootstrap}
        className="mt-3 px-4 py-2 bg-blue-600 text-white text-sm rounded-lg hover:bg-blue-700 transition-colors"
      >
        Set Up Secure User
      </button>
    </div>
  );
}

function BootstrapProgress({ result }: { result: BootstrapResult }) {
  const stepLabels: Record<string, string> = {
    create_policy: "Create IAM policy",
    create_user: "Create IAM user",
    attach_policy: "Attach policy to user",
    create_access_key: "Create access key",
    validate_new_credentials: "Validate new credentials",
    delete_source_key: "Delete source access key",
    write_config: "Save configuration",
  };

  function stepIcon(step: BootstrapStep): string {
    switch (step.status) {
      case "succeeded":
        return "✅";
      case "failed":
        return "❌";
      case "in_progress":
        return "⏳";
      case "pending":
        return "⬜";
      default:
        return "·";
    }
  }

  return (
    <div className="bg-gray-50 border border-gray-200 rounded-lg p-4 mt-4">
      <p className="text-sm font-medium text-gray-700 mb-3">Bootstrap Progress</p>
      <div className="space-y-2">
        {result.steps.map((step, i) => (
          <div key={i} className="flex items-start gap-2 text-sm">
            <span className="flex-shrink-0">{stepIcon(step)}</span>
            <div className="min-w-0">
              <span className="text-gray-800">
                {stepLabels[step.name] ?? step.name}
              </span>
              {step.detail && (
                <p className="text-xs text-gray-500 mt-0.5 truncate">
                  {step.detail}
                </p>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function KeyLimitCard({
  keys,
  deletingKeyId,
  onDelete,
  onCancel,
}: {
  keys: AccessKeyInfo[];
  deletingKeyId: string | null;
  onDelete: (keyId: string) => void;
  onCancel: () => void;
}) {
  return (
    <div className="bg-amber-50 border border-amber-200 rounded-lg p-4 mt-4">
      <p className="text-amber-900 text-sm font-medium mb-2">
        Access key limit reached
      </p>
      <p className="text-amber-800 text-sm mb-4">
        The <code className="text-xs bg-amber-100 px-1 rounded">claria-admin</code> user
        already has 2 access keys (the AWS maximum). Choose one to delete so Claria can
        create a new one.
      </p>

      {keys.length === 0 ? (
        <p className="text-amber-700 text-sm flex items-center gap-2">
          <Spinner /> Loading existing keys...
        </p>
      ) : (
        <div className="space-y-3">
          {keys.map((key) => (
            <div
              key={key.access_key_id}
              className="bg-white border border-amber-200 rounded-lg p-3 flex items-center justify-between gap-4"
            >
              <div className="min-w-0">
                <p className="text-sm font-mono text-gray-800">
                  {key.access_key_id.slice(0, 8)}...{key.access_key_id.slice(-4)}
                </p>
                <div className="flex flex-wrap gap-x-4 gap-y-0.5 mt-1">
                  <span className={`text-xs ${
                    key.status === "Active" ? "text-green-700" : "text-gray-500"
                  }`}>
                    {key.status}
                  </span>
                  {key.created_at && (
                    <span className="text-xs text-gray-500">
                      Created: {formatDate(key.created_at)}
                    </span>
                  )}
                  <span className="text-xs text-gray-500">
                    {key.last_used_at
                      ? `Last used: ${formatDate(key.last_used_at)}${
                          key.last_used_service ? ` (${key.last_used_service})` : ""
                        }`
                      : "Never used"}
                  </span>
                </div>
              </div>
              <button
                onClick={() => onDelete(key.access_key_id)}
                disabled={deletingKeyId !== null}
                className="flex-shrink-0 px-3 py-1.5 text-sm text-red-700 bg-red-50 border border-red-200 rounded-lg hover:bg-red-100 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {deletingKeyId === key.access_key_id ? (
                  <span className="flex items-center gap-1.5">
                    <Spinner /> Deleting...
                  </span>
                ) : (
                  "Delete this key"
                )}
              </button>
            </div>
          ))}
        </div>
      )}

      <button
        onClick={onCancel}
        disabled={deletingKeyId !== null}
        className="mt-4 text-sm text-amber-700 hover:text-amber-900 disabled:opacity-50"
      >
        Cancel and start over
      </button>
    </div>
  );
}

/** Format an ISO 8601 / AWS date string to a short readable form. */
function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString(undefined, {
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