import { useState, useEffect } from "react";
import StepIndicator from "../components/StepIndicator";
import {
  saveConfig,
  validateCredentials,
  listAwsProfiles,
  type CredentialSource,
  type CallerIdentity,
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

type Mode = "inline" | "profile" | "default_chain";

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
  const [validating, setValidating] = useState(false);
  const [validated, setValidated] = useState<CallerIdentity | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    listAwsProfiles()
      .then(setProfiles)
      .catch(() => setProfiles([]));
  }, []);

  const systemNameValid = /^[a-z0-9][a-z0-9-]{1,38}[a-z0-9]$/.test(systemName);

  function buildCredentials(): CredentialSource {
    switch (mode) {
      case "inline":
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

  async function handleValidate() {
    setValidating(true);
    setError(null);
    setValidated(null);
    try {
      const identity = await validateCredentials(region, buildCredentials());
      setValidated(identity);
    } catch (e) {
      setError(String(e));
    } finally {
      setValidating(false);
    }
  }

  async function handleSave() {
    if (!validated) return;
    setSaving(true);
    setError(null);
    try {
      await saveConfig(region, systemName, buildCredentials());
      navigate("scan");
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  const canValidate =
    systemNameValid &&
    (mode === "inline"
      ? accessKeyId.length > 0 && secretAccessKey.length > 0
      : mode === "profile"
        ? profileName.length > 0
        : true);

  return (
    <div className="max-w-2xl mx-auto p-8">
      <StepIndicator current={3} />

      <h2 className="text-2xl font-bold mb-6">Step 3: Configure Credentials</h2>

      {/* Mode selector */}
      <div className="flex gap-2 mb-6">
        {[
          { value: "inline" as Mode, label: "I'm new to AWS" },
          { value: "profile" as Mode, label: "Existing AWS profile" },
          { value: "default_chain" as Mode, label: "Default credentials" },
        ].map(({ value, label }) => (
          <button
            key={value}
            onClick={() => {
              setMode(value);
              setValidated(null);
              setError(null);
            }}
            className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
              mode === value
                ? "bg-blue-500 text-white"
                : "bg-white text-gray-700 border border-gray-300 hover:bg-gray-50"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      <div className="space-y-4">
        {/* Region */}
        <div>
          <label className="block text-sm font-medium text-gray-700 mb-1">
            AWS Region
          </label>
          <select
            value={region}
            onChange={(e) => setRegion(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 rounded-lg bg-white"
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
            onChange={(e) => setSystemName(e.target.value.toLowerCase())}
            placeholder="claria"
            className={`w-full px-3 py-2 border rounded-lg ${
              systemName.length > 0 && !systemNameValid
                ? "border-red-300"
                : "border-gray-300"
            }`}
          />
          <p className="text-xs text-gray-500 mt-1">
            Lowercase letters, numbers, and hyphens. 3-40 characters.
          </p>
        </div>

        {/* Inline mode fields */}
        {mode === "inline" && (
          <>
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Access Key ID
              </label>
              <input
                type="text"
                value={accessKeyId}
                onChange={(e) => setAccessKeyId(e.target.value)}
                placeholder="AKIAIOSFODNN7EXAMPLE"
                className="w-full px-3 py-2 border border-gray-300 rounded-lg font-mono text-sm"
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
                  onChange={(e) => setSecretAccessKey(e.target.value)}
                  placeholder="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
                  className="w-full px-3 py-2 border border-gray-300 rounded-lg font-mono text-sm pr-16"
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

        {/* Profile mode fields */}
        {mode === "profile" && (
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">
              Profile Name
            </label>
            {profiles.length > 0 ? (
              <select
                value={profileName}
                onChange={(e) => setProfileName(e.target.value)}
                className="w-full px-3 py-2 border border-gray-300 rounded-lg bg-white"
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
                onChange={(e) => setProfileName(e.target.value)}
                placeholder="claria-admin"
                className="w-full px-3 py-2 border border-gray-300 rounded-lg"
              />
            )}
          </div>
        )}

        {/* Default chain info */}
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

      {/* Validation result */}
      {validated && (
        <div className="bg-green-50 border border-green-200 rounded-lg p-4 mt-6">
          <p className="text-green-800 text-sm font-medium mb-1">
            Credentials verified
          </p>
          <p className="text-green-700 text-xs font-mono">
            Account: {validated.account_id}
          </p>
          <p className="text-green-700 text-xs font-mono">
            ARN: {validated.arn}
          </p>
        </div>
      )}

      {error && (
        <div className="bg-red-50 border border-red-200 rounded-lg p-4 mt-6">
          <p className="text-red-800 text-sm">{error}</p>
        </div>
      )}

      {/* Actions */}
      <div className="flex justify-between mt-8">
        <button
          onClick={() => navigate("guide-iam")}
          className="px-4 py-2 text-gray-600 hover:text-gray-800"
        >
          Back
        </button>
        <div className="flex gap-3">
          <button
            onClick={handleValidate}
            disabled={!canValidate || validating}
            className="px-6 py-2 bg-white text-gray-700 border border-gray-300 rounded-lg hover:bg-gray-50 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {validating ? "Validating..." : "Validate"}
          </button>
          <button
            onClick={handleSave}
            disabled={!validated || saving}
            className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {saving ? "Saving..." : "Save & Continue"}
          </button>
        </div>
      </div>
    </div>
  );
}
