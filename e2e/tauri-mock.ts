/**
 * Tauri IPC mock for E2E tests.
 *
 * Unlike the screenshot mock (which returns static fixtures), this mock
 * maintains mutable state and talks to claria-mock-aws for AWS-dependent
 * commands. This lets us test the real onboarding flow end-to-end.
 */

const MOCK_AWS_URL = "http://127.0.0.1:9000";

/**
 * Build a string of JS that will be injected into the browser via
 * `page.addInitScript()`. All state lives inside the closure so each
 * test gets a fresh environment.
 */
export function buildInitScript(): string {
  return `
    (() => {
      const MOCK_AWS_URL = "${MOCK_AWS_URL}";

      // ── Mutable app state ──────────────────────────────────────────────
      let configSaved = false;
      let savedConfig = null;

      // ── Plan / apply fixtures built from mock-aws scan ─────────────────
      // We generate "create" entries for a fresh account so the UI shows
      // a meaningful plan, then "ok" entries after apply.
      let appliedOnce = false;

      function freshPlanEntries() {
        const specs = [
          { resource_type: "iam_user", resource_name: "claria-admin", label: "IAM User", description: "Dedicated least-privilege user", severity: "info", credential_scope: "elevated", iam_actions: [] },
          { resource_type: "iam_user_policy", resource_name: "claria-admin-policy", label: "IAM Policy", description: "Permissions scoped to only what Claria needs", severity: "normal", credential_scope: "elevated", iam_actions: [] },
          { resource_type: "baa_agreement", resource_name: "aws-baa", label: "BAA Agreement", description: "Business Associate Agreement", severity: "elevated", credential_scope: "elevated", iam_actions: [] },
          { resource_type: "s3_bucket", resource_name: "185735714230-claria-data", label: "S3 Bucket", description: "Encrypted storage for client records", severity: "normal", credential_scope: "regular", iam_actions: [] },
          { resource_type: "s3_bucket_versioning", resource_name: "185735714230-claria-data", label: "S3 Bucket Versioning", description: "Protects against accidental deletion", severity: "normal", credential_scope: "regular", iam_actions: [] },
          { resource_type: "s3_bucket_encryption", resource_name: "185735714230-claria-data", label: "S3 Bucket Encryption", description: "Data encrypted at rest", severity: "normal", credential_scope: "regular", iam_actions: [] },
          { resource_type: "s3_bucket_public_access", resource_name: "185735714230-claria-data", label: "S3 Public Access Block", description: "All public access blocked", severity: "normal", credential_scope: "regular", iam_actions: [] },
          { resource_type: "s3_bucket_policy", resource_name: "185735714230-claria-data", label: "S3 Bucket Policy", description: "Enforces TLS-only access", severity: "normal", credential_scope: "regular", iam_actions: [] },
          { resource_type: "cloudtrail_trail", resource_name: "claria-audit-trail", label: "CloudTrail Trail", description: "Audit log for all S3 access", severity: "normal", credential_scope: "regular", iam_actions: [] },
          { resource_type: "cloudtrail_s3_events", resource_name: "claria-audit-trail", label: "CloudTrail S3 Events", description: "Object-level logging", severity: "normal", credential_scope: "regular", iam_actions: [] },
          { resource_type: "bedrock_model_access", resource_name: "anthropic.claude-sonnet-4-20250514-v1:0", label: "Bedrock Model Access", description: "Claude Sonnet 4", severity: "elevated", credential_scope: "elevated", iam_actions: [] },
          { resource_type: "bedrock_model_access", resource_name: "anthropic.claude-haiku-4-5-20251001-v1:0", label: "Bedrock Model Access", description: "Claude Haiku 4.5", severity: "elevated", credential_scope: "elevated", iam_actions: [] },
          { resource_type: "bedrock_model_access", resource_name: "anthropic.claude-opus-4-6-20260301-v1:0", label: "Bedrock Model Access", description: "Claude Opus 4.6", severity: "elevated", credential_scope: "elevated", iam_actions: [] },
        ];

        if (appliedOnce) {
          return specs.map(spec => ({
            spec: { ...spec, lifecycle: "managed", desired: {} },
            action: "ok",
            cause: "in_sync",
            drift: [],
            actual: null,
          }));
        }

        return specs.map(spec => ({
          spec: { ...spec, lifecycle: "managed", desired: {} },
          action: "create",
          cause: "first_provision",
          drift: [],
          actual: null,
        }));
      }

      // ── transformCallback support for Tauri Channel ──────────────────
      // The Tauri SDK's Channel constructor calls transformCallback to
      // register a callback on the window. We provide a minimal impl.
      let _callbackId = 0;

      window.__TAURI_INTERNALS__ = {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },

        transformCallback: function(callback, once) {
          const id = _callbackId++;
          const key = "_" + id;
          window[key] = (data) => {
            if (once) delete window[key];
            if (callback) callback(data);
          };
          return id;
        },

        invoke: async function(cmd, args) {
          // ── Tauri plugin stubs ───────────────────────────────────────
          if (cmd === "plugin:app|version") return "0.15.0";
          if (cmd === "plugin:app|name") return "Claria";
          if (cmd === "plugin:app|tauri_version") return "2.0.0";
          if (cmd === "plugin:event|listen") return 0;
          if (cmd === "plugin:event|unlisten") return;
          if (cmd === "plugin:webview|get_all_webviews") {
            return [{ label: "main", url: "http://localhost:1420" }];
          }

          // ── Config persistence (mutable in-memory) ───────────────────
          if (cmd === "has_config") return configSaved;

          if (cmd === "load_config") {
            if (!configSaved || !savedConfig) throw "No config found";
            return savedConfig;
          }

          if (cmd === "save_config") {
            savedConfig = {
              region: args.region,
              system_name: args.systemName,
              account_id: args.accountId,
              created_at: new Date().toISOString(),
              credential_type: args.credentials?.type ?? "inline",
              profile_name: args.credentials?.profile_name ?? null,
              access_key_hint: args.credentials?.access_key_id
                ? args.credentials.access_key_id.slice(0, 4) + "..." + args.credentials.access_key_id.slice(-4)
                : null,
              preferred_model_id: null,
              cost_explorer_enabled: false,
              hourly_cost_data: false,
            };
            configSaved = true;
            return null;
          }

          if (cmd === "delete_config") {
            configSaved = false;
            savedConfig = null;
            return null;
          }

          // ── Credential assessment ────────────────────────────────────
          if (cmd === "assess_credentials") {
            // Always classify as root for the fresh-account E2E test
            return {
              identity: {
                account_id: "185735714230",
                arn: "arn:aws:iam::185735714230:root",
                user_id: "185735714230",
                is_root: true,
              },
              credential_class: "root",
              reason: "Authenticated as the root user of account 185735714230.",
            };
          }

          // ── First-run provisioning (unified scan/apply) ──────────────
          if (cmd === "provision_scan") {
            return {
              entries: freshPlanEntries(),
              needs_escalation: !appliedOnce,
              account_id: "185735714230",
            };
          }

          if (cmd === "provision_apply") {
            appliedOnce = true;
            // The real command bootstraps the IAM user and writes config.
            savedConfig = {
              region: args.region,
              system_name: args.systemName,
              account_id: "185735714230",
              created_at: new Date().toISOString(),
              credential_type: "inline",
              profile_name: null,
              access_key_hint: "AKIA...0001",
              preferred_model_id: null,
              cost_explorer_enabled: false,
              hourly_cost_data: false,
            };
            configSaved = true;
            return { entries: freshPlanEntries(), access_key_limit: null };
          }

          // ── Plan (provisioner scan) ──────────────────────────────────
          if (cmd === "plan") {
            const entries = freshPlanEntries();
            // Skip progress events — Channel internals are too complex to mock.
            // The Provision page will jump straight from "scanning" to "planned".
            return entries;
          }

          // ── Apply (provisioner execute) ──────────────────────────────
          if (cmd === "apply") {
            appliedOnce = true;
            // Return all-ok entries after apply
            return freshPlanEntries();
          }

          // ── List AWS profiles ────────────────────────────────────────
          if (cmd === "list_aws_profiles") return [];

          // ── Chat models ──────────────────────────────────────────────
          if (cmd === "list_chat_models") {
            return [
              { model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0", name: "Claude Opus 4.6" },
              { model_id: "us.anthropic.claude-sonnet-4-20250514-v1:0", name: "Claude Sonnet 4" },
              { model_id: "us.anthropic.claude-haiku-4-5-20251001-v1:0", name: "Claude Haiku 4.5" },
            ];
          }

          // ── Client operations (post-onboarding) ──────────────────────
          if (cmd === "list_clients") return [];
          if (cmd === "create_client") {
            return {
              id: "e2e-test-client-" + Date.now(),
              name: args.name,
              created_at: new Date().toISOString(),
            };
          }

          // ── Record files ─────────────────────────────────────────────
          if (cmd === "list_record_files") return [];
          if (cmd === "list_record_context") return [];
          if (cmd === "list_deleted_files") return [];
          if (cmd === "list_deleted_clients") return [];

          // ── Update check ─────────────────────────────────────────────
          if (cmd === "check_for_updates") {
            return {
              current_version: "0.15.0",
              latest_version: "0.15.0",
              update_available: false,
              release_url: "",
            };
          }

          // ── Provisioner state reset ──────────────────────────────────
          if (cmd === "reset_provisioner_state") return null;

          // ── Whisper ──────────────────────────────────────────────────
          if (cmd === "get_whisper_models") return [];

          // ── Prompts ──────────────────────────────────────────────────
          if (cmd === "get_prompt") return "";
          if (cmd === "list_prompt_versions") return [];

          // ── Cost explorer ────────────────────────────────────────────
          if (cmd === "probe_cost_explorer") return false;

          console.warn("[tauri-mock-e2e] unhandled command:", cmd, args);
          return null;
        },

        convertFileSrc: function(path) {
          return path;
        },
      };
    })();
  `;
}
