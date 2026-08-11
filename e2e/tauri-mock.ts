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
export function buildInitScript(
  options: { configured?: boolean } = {},
): string {
  const configured = options.configured === true;
  return `
    (() => {
      const MOCK_AWS_URL = "${MOCK_AWS_URL}";

      // ── Mutable app state ──────────────────────────────────────────────
      let configSaved = ${configured};
      let savedConfig = ${configured ? `{
        region: "us-east-1",
        system_name: "claria",
        account_id: "185735714230",
        created_at: "2026-08-01T00:00:00Z",
        credential_type: "inline",
        profile_name: null,
        access_key_hint: "AKIA...0001",
        preferred_model_id: "us.anthropic.claude-sonnet-4-20250514-v1:0",
        cost_explorer_enabled: false,
        hourly_cost_data: false,
        prompt_caching_enabled: true,
        transcription: {
          default_language: "english",
          default_speaker_count: 2,
          use_medical_for_english: false,
          translate_to_english: false,
        },
        report_authoring: {
          max_tool_rounds: 40,
          max_converse_calls: 50,
          max_tool_uses_per_response: 80,
          max_retained_turns: 200,
        },
      }` : "null"};
      window.__REPORT_COMMANDS__ = [];
      window.__REPORT_INVOCATIONS__ = [];
      window.__CHAT_COMMANDS__ = [];
      let clientName = "Jane Doe";
      const clientNameHistory = [
        { name: "Jane Doe", changed_at: "2026-08-01T12:00:00Z" },
        { name: "Jane A. Doe", changed_at: "2026-07-10T15:30:00Z" },
      ];
      const toolActivity = (name, toolUseId, summary, input, result) => ({
        kind: "tool_activity",
        name,
        summary,
        status: "succeeded",
        invocation_json: JSON.stringify({ toolUse: { toolUseId, name, input } }, null, 2),
        result_json: JSON.stringify({
          toolResult: {
            toolUseId,
            status: "success",
            content: [{ json: result }],
          },
        }, null, 2),
        created_at: new Date().toISOString(),
      });
      let reportWorkspace = {
        schema_version: 5,
        session_name: "Writer Session (1)",
        report_id: "99999999-9999-4999-8999-999999999999",
        client_id: "aaaaaaaa-1111-4222-8333-bbbbbbbbbbbb",
        draft: {
          revision: 0,
          content: { title: "Untitled report", sections: [] },
          created_at: "2026-08-01T00:00:00Z",
          updated_at: "2026-08-01T00:00:00Z",
          last_applied_proposal_id: null,
        },
        turns: [],
        pending_proposal: null,
        resolutions: [],
        last_agent_revision: null,
        last_export: null,
        template_import: null,
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
      };
      let reportSessionStarted = false;
      const reportWorkspaces = new Map();
      const rememberActiveReport = () => {
        if (reportSessionStarted) {
          reportWorkspaces.set(reportWorkspace.report_id, structuredClone(reportWorkspace));
        }
      };
      const freshReportWorkspace = (ordinal) => {
        const fresh = structuredClone(reportWorkspace);
        fresh.report_id = "99999999-9999-4999-8999-" + String(ordinal).padStart(12, "0");
        fresh.session_name = "Writer Session (" + ordinal + ")";
        fresh.draft = {
          revision: 0,
          content: { title: "Untitled report", sections: [] },
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
          last_applied_proposal_id: null,
        };
        fresh.turns = [];
        fresh.pending_proposal = null;
        fresh.resolutions = [];
        fresh.last_agent_revision = null;
        fresh.last_export = null;
        fresh.template_import = null;
        fresh.created_at = new Date().toISOString();
        fresh.updated_at = fresh.created_at;
        return fresh;
      };
      const reportDraftRevisions = new Map([
        [reportWorkspace.draft.revision, structuredClone(reportWorkspace.draft)],
      ]);
      const rememberReportDraft = () => {
        reportDraftRevisions.set(
          reportWorkspace.draft.revision,
          structuredClone(reportWorkspace.draft),
        );
      };
      const reportTemplatePreview = {
        import_id: "77777777-7777-4777-8777-777777777777",
        content: {
          title: "Imported Evaluation Template",
          sections: [{
            id: "66666666-6666-4666-8666-666666666666",
            heading: "Assessment Scores",
            blocks: [{
              kind: "table",
              rows: [["Measure", "Score"], ["Attention", "{{score}}"]],
              has_header: true,
              column_widths: [7000, 3000],
            }],
          }],
        },
        warnings: [{
          code: "headers_footers_omitted",
          message: "Headers or footers were omitted.",
          count: 2,
        }],
        stats: { sections: 1, paragraphs: 0, bullet_lists: 0, tables: 1, table_cells: 4, placeholder_count: 1 },
      };

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
              prompt_caching_enabled: true,
              transcription: {
                default_language: "english",
                default_speaker_count: 2,
                use_medical_for_english: false,
                translate_to_english: false,
              },
              report_authoring: {
                max_tool_rounds: 40,
                max_converse_calls: 50,
                max_tool_uses_per_response: 80,
                max_retained_turns: 200,
              },
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
              prompt_caching_enabled: true,
              transcription: {
                default_language: "english",
                default_speaker_count: 2,
                use_medical_for_english: false,
                translate_to_english: false,
              },
              report_authoring: {
                max_tool_rounds: 40,
                max_converse_calls: 50,
                max_tool_uses_per_response: 80,
                max_retained_turns: 200,
              },
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
          if (cmd === "list_clients") {
            return ${configured}
              ? [{
                  id: "aaaaaaaa-1111-4222-8333-bbbbbbbbbbbb",
                  name: clientName,
                  created_at: "2026-08-01T12:00:00Z",
                }]
              : [];
          }
          if (cmd === "create_client") {
            return {
              id: "e2e-test-client-" + Date.now(),
              name: args.name,
              created_at: new Date().toISOString(),
            };
          }
          if (cmd === "get_client_record_details") {
            return {
              id: args.clientId,
              name: clientName,
              created_at: "2026-08-01T12:00:00Z",
              updated_at: "2026-08-01T12:00:00Z",
              file_count: 3,
              storage_bytes: 5767168,
              storage_bytes_with_history: 7340032,
              name_history: structuredClone(clientNameHistory),
            };
          }
          if (cmd === "update_client_name") {
            clientName = args.name.trim();
            const updatedAt = "2026-08-02T12:30:00Z";
            clientNameHistory.unshift({ name: clientName, changed_at: updatedAt });
            return {
              id: args.clientId,
              name: clientName,
              updated_at: updatedAt,
            };
          }

          // ── Record files and unchanged Chat workflow ─────────────────
          if (cmd === "list_record_files") return [{
            filename: "chat-history/77777777-7777-4777-8777-777777777777.json",
            size: 842,
            uploaded_at: "2026-08-01T12:00:00Z",
          }];
          if (cmd === "list_record_context") return [];
          if (cmd === "get_record_file_text") {
            const previews = {
              "intake-parent-interview.txt": "Parent interview record used for the complete report.",
              "teacher-observation.txt": "Teacher observation record used for the complete report.",
              "assessment-scores.json": '{"attention": "needs support"}',
            };
            return previews[args.filename] ?? "Record preview unavailable.";
          }
          if (cmd === "list_deleted_files") return [];
          if (cmd === "list_deleted_clients") return [];
          if (cmd === "lookup_model_pricing") return {
            input_per_million: 3,
            output_per_million: 15,
            cache_read_per_million: 0.3,
            cache_write_per_million: 3.75,
            cache_write_1h_per_million: 6,
          };
          if (cmd === "list_chat_histories") return [{
            chat_id: "77777777-7777-4777-8777-777777777777",
            filename: "chat-history/77777777-7777-4777-8777-777777777777.json",
            name: "Chat (1)",
            size: 842,
            updated_at: "2026-08-01T12:00:00Z",
          }];
          if (cmd === "load_chat_history") {
            window.__CHAT_COMMANDS__.push({ cmd, args: structuredClone(args) });
            return {
              chat_id: args.chatId,
              name: "Chat (1)",
              model_id: "us.anthropic.claude-sonnet-4-20250514-v1:0",
              messages: [
                { role: "user", content: "Earlier question", timestamp: "2026-08-01T11:59:00Z", usage: null },
                { role: "assistant", content: "Earlier answer", timestamp: "2026-08-01T12:00:00Z", usage: { model_id: "us.anthropic.claude-sonnet-4-20250514-v1:0", input_tokens: 20, output_tokens: 5, cache_read_input_tokens: 0, cache_write_input_tokens: 0, cache_ttl: null, cost_usd: 0.0001, pricing_version: 4 } },
              ],
              created_at: "2026-08-01T12:00:00Z",
              updated_at: "2026-08-01T12:00:00Z",
            };
          }
          if (cmd === "chat_message") {
            window.__CHAT_COMMANDS__.push({ cmd, args: structuredClone(args) });
            return {
              chat_id: args.chatId || "77777777-7777-4777-8777-777777777777",
              chat_name: "Chat (1)",
              content: "Unchanged Chat response",
              usage: { model_id: args.modelId, input_tokens: 3, output_tokens: 60, cache_read_input_tokens: 4243, cache_write_input_tokens: 5000, cache_ttl: "five_minutes", cost_usd: 0.0209319, pricing_version: 4 },
            };
          }

          // ── Writing assistant ────────────────────────────────────────
          if (cmd === "list_editor_history") {
            rememberActiveReport();
            return Array.from(reportWorkspaces.values())
              .filter((workspace) => workspace.turns.length > 0 || workspace.draft.revision > 0 || workspace.template_import)
              .sort((left, right) => right.updated_at.localeCompare(left.updated_at))
              .map((workspace) => ({
                report_id: workspace.report_id,
                name: workspace.session_name,
                title: workspace.draft.content.title,
                revision: workspace.draft.revision,
                turn_count: workspace.turns.length,
                updated_at: workspace.updated_at,
                last_export: workspace.last_export,
              }));
          }
          if (cmd === "start_report_workspace") {
            window.__REPORT_COMMANDS__.push(cmd);
            window.__REPORT_INVOCATIONS__.push({ cmd, args: structuredClone(args) });
            rememberActiveReport();
            const existing = reportWorkspaces.get(args.reportId);
            if (existing) {
              reportWorkspace = structuredClone(existing);
            } else if (!reportSessionStarted) {
              reportWorkspace.report_id = args.reportId;
            } else {
              reportWorkspace = freshReportWorkspace(reportWorkspaces.size + 1);
              reportWorkspace.report_id = args.reportId;
              reportDraftRevisions.clear();
              reportDraftRevisions.set(0, structuredClone(reportWorkspace.draft));
            }
            reportSessionStarted = true;
            rememberActiveReport();
            return structuredClone(reportWorkspace);
          }
          if (cmd === "load_report_workspace") {
            window.__REPORT_COMMANDS__.push(cmd);
            window.__REPORT_INVOCATIONS__.push({ cmd, args: structuredClone(args) });
            rememberActiveReport();
            const resumed = reportWorkspaces.get(args.reportId);
            if (!resumed) throw "That Writing session is no longer available.";
            reportWorkspace = structuredClone(resumed);
            reportSessionStarted = true;
            return structuredClone(reportWorkspace);
          }
          if (cmd === "list_report_revisions") {
            window.__REPORT_COMMANDS__.push(cmd);
            if (args.reportId !== reportWorkspace.report_id) throw "The report changed on another computer. Reload it before continuing.";
            return Array.from(reportDraftRevisions.values())
              .sort((left, right) => right.revision - left.revision)
              .map((draft) => ({
                revision: draft.revision,
                title: draft.content.title,
                updated_at: draft.updated_at,
              }));
          }
          if (cmd === "load_report_revision") {
            window.__REPORT_COMMANDS__.push(cmd);
            if (args.reportId !== reportWorkspace.report_id) throw "The report changed on another computer. Reload it before continuing.";
            const draft = reportDraftRevisions.get(args.revision);
            if (!draft) throw "That report revision is no longer available.";
            return structuredClone(draft);
          }
          if (cmd === "revert_report_revision") {
            window.__REPORT_COMMANDS__.push(cmd);
            window.__REPORT_INVOCATIONS__.push({ cmd, args: structuredClone(args) });
            if (args.reportId !== reportWorkspace.report_id || args.expectedRevision !== reportWorkspace.draft.revision) throw "The report changed on another computer. Reload it before continuing.";
            if (args.revision >= args.expectedRevision) throw "Choose an earlier report revision to restore.";
            const historical = reportDraftRevisions.get(args.revision);
            if (!historical) throw "That report revision is no longer available.";
            reportWorkspace = {
              ...reportWorkspace,
              draft: {
                ...reportWorkspace.draft,
                revision: reportWorkspace.draft.revision + 1,
                content: structuredClone(historical.content),
                updated_at: new Date().toISOString(),
              },
              template_import: reportWorkspace.template_import
                ? { ...reportWorkspace.template_import, reviewed_revision: reportWorkspace.draft.revision + 1, review_required: false }
                : null,
            };
            rememberReportDraft();
            return structuredClone(reportWorkspace);
          }
          if (cmd === "save_report_draft") {
            window.__REPORT_COMMANDS__.push(cmd);
            window.__REPORT_INVOCATIONS__.push({ cmd, args: structuredClone(args) });
            if (args.reportId !== reportWorkspace.report_id || args.expectedRevision !== reportWorkspace.draft.revision) throw "The report changed on another computer. Reload it before continuing.";
            if (reportWorkspace.pending_proposal) throw "Accept or reject the pending proposal before editing the report.";
            reportWorkspace = {
              ...reportWorkspace,
              draft: {
                ...reportWorkspace.draft,
                revision: reportWorkspace.draft.revision + 1,
                content: {
                  title: args.draft.title,
                  sections: args.draft.sections.map((section, index) => ({
                    ...section,
                    id: section.id || "10000000-0000-4000-8000-" + String(index + 1).padStart(12, "0"),
                  })),
                },
                updated_at: new Date().toISOString(),
              },
              template_import: reportWorkspace.template_import
                ? { ...reportWorkspace.template_import, reviewed_revision: reportWorkspace.draft.revision + 1, review_required: false }
                : null,
            };
            rememberReportDraft();
            return reportWorkspace;
          }
          if (cmd === "discard_queued_report_edits") {
            window.__REPORT_COMMANDS__.push(cmd);
            window.__REPORT_INVOCATIONS__.push({ cmd, args: structuredClone(args) });
            if (args.reportId !== reportWorkspace.report_id || args.expectedRevision !== reportWorkspace.draft.revision) throw "The report changed on another computer. Reload it before continuing.";
            const baseline = reportDraftRevisions.get(reportWorkspace.last_agent_revision || 0);
            if (!baseline) throw "The queued report baseline is no longer available.";
            const nextRevision = reportWorkspace.draft.revision + 1;
            reportWorkspace = {
              ...reportWorkspace,
              draft: {
                ...reportWorkspace.draft,
                revision: nextRevision,
                content: structuredClone(baseline.content),
                updated_at: new Date().toISOString(),
              },
              last_agent_revision: nextRevision,
              updated_at: new Date().toISOString(),
            };
            rememberReportDraft();
            return structuredClone(reportWorkspace);
          }
          if (cmd === "list_writer_templates") return [{
            id: "55555555-5555-4555-8555-555555555555",
            name: "Imported Evaluation Template",
            size: 24576,
            uploaded_at: "2026-08-01T12:00:00Z",
            use_count: 0,
          }];
          if (cmd === "preview_writer_template") {
            window.__REPORT_COMMANDS__.push(cmd);
            window.__REPORT_INVOCATIONS__.push({ cmd, args: structuredClone(args) });
            return structuredClone(reportTemplatePreview);
          }
          if (cmd === "apply_report_template") {
            window.__REPORT_COMMANDS__.push(cmd);
            window.__REPORT_INVOCATIONS__.push({ cmd, args: structuredClone(args) });
            if (args.importId !== reportTemplatePreview.import_id) throw "That template preview expired. Select the template again.";
            if (args.reportId !== reportWorkspace.report_id || args.expectedRevision !== reportWorkspace.draft.revision) throw "The report changed on another computer. Reload it before continuing.";
            reportWorkspace = {
              ...reportWorkspace,
              draft: {
                ...reportWorkspace.draft,
                revision: reportWorkspace.draft.revision + 1,
                content: structuredClone(reportTemplatePreview.content),
                updated_at: new Date().toISOString(),
              },
              template_import: {
                writer_template_id: "55555555-5555-4555-8555-555555555555",
                writer_template_name: "Imported Evaluation Template",
                imported_revision: reportWorkspace.draft.revision + 1,
                imported_at: new Date().toISOString(),
                warnings: structuredClone(reportTemplatePreview.warnings),
                reviewed_revision: reportWorkspace.draft.revision + 1,
                review_required: false,
                placeholder_count: reportTemplatePreview.stats.placeholder_count,
              },
            };
            rememberReportDraft();
            return structuredClone(reportWorkspace);
          }
          if (cmd === "discard_report_template_preview") {
            window.__REPORT_COMMANDS__.push(cmd);
            return null;
          }
          if (cmd === "generate_full_report") {
            window.__REPORT_COMMANDS__.push(cmd);
            window.__REPORT_INVOCATIONS__.push({ cmd, args: structuredClone(args) });
            if (args.reportId !== reportWorkspace.report_id || args.expectedRevision !== reportWorkspace.draft.revision) throw "The report changed on another computer. Reload it before continuing.";
            if (reportWorkspace.pending_proposal) throw "Accept or reject the pending proposal before starting another writer action.";
            const nextRevision = reportWorkspace.draft.revision + 1;
            const now = new Date().toISOString();
            reportWorkspace = {
              ...reportWorkspace,
              draft: {
                ...reportWorkspace.draft,
                revision: nextRevision,
                content: {
                  title: "Complete Generated Evaluation",
                  sections: [{
                    id: "11111111-2222-4333-8444-555555555555",
                    heading: "Summary",
                    blocks: [{ kind: "paragraph", text: "Complete generated report content from every readable record." }],
                  }],
                },
                updated_at: now,
              },
              turns: [...reportWorkspace.turns, {
                id: "turn-full-1",
                model_id: args.modelId,
                timeline: [
                  { kind: "message", role: "user", text: "Fill the complete report from all readable client records.\\n\\nGuidance: " + args.guidance, created_at: now },
                  toolActivity("write_full_draft_section", "section-full-1", "Staged complete report section", { section_id: null, position: 0, block_count: 1, content_retained: false }, { status: "section_staged", section_count: 1 }),
                  toolActivity("finish_full_draft", "finish-full-1", "Finalized complete working draft", { summary_retained: false }, { status: "full_draft_finalized", section_count: 1 }),
                  { kind: "message", role: "assistant", text: "The complete working draft is ready.", created_at: now },
                ],
                usage: { model_id: args.modelId, input_tokens: 10, output_tokens: 80, cache_read_input_tokens: 4200, cache_write_input_tokens: 3600, cache_ttl: null, cost_usd: 0.01599, pricing_version: 5 },
                usage_complete: true,
                converse_calls: 3,
                tool_uses: 3,
                context_files: [
                  { filename: "intake-parent-interview.txt", available: true },
                  { filename: "teacher-observation.txt", available: true },
                  { filename: "assessment-scores.json", available: true },
                ],
                context_reads: [],
                created_at: now,
                completed_at: now,
              }],
              pending_proposal: null,
              last_agent_revision: nextRevision,
              updated_at: now,
            };
            rememberReportDraft();
            return {
              workspace: structuredClone(reportWorkspace),
              turn_id: "turn-full-1",
              attempt_id: "attempt-full-1",
              assistant_text: "The complete working draft is ready.",
              usage: { model_id: args.modelId, input_tokens: 10, output_tokens: 80, cache_read_input_tokens: 4200, cache_write_input_tokens: 3600, cache_ttl: null, cost_usd: 0.01599, pricing_version: 5 },
              usage_complete: true,
              converse_calls: 3,
              tool_uses: 3,
              included_record_files: 3,
              unavailable_record_files: 0,
              record_characters: 6400,
            };
          }
          if (cmd === "send_report_message") {
            window.__REPORT_COMMANDS__.push(cmd);
            window.__REPORT_INVOCATIONS__.push({ cmd, args: structuredClone(args) });
            if (args.reportId !== reportWorkspace.report_id || args.expectedRevision !== reportWorkspace.draft.revision) throw "The report changed on another computer. Reload it before continuing.";
            if (reportWorkspace.pending_proposal) throw "Accept or reject the pending proposal before sending another instruction.";
            const proposedContent = {
              title: "Comprehensive Evaluation",
              sections: [{
                id: "11111111-2222-4333-8444-555555555555",
                heading: "Summary",
                blocks: [
                  { kind: "paragraph", text: "Jane was referred for an evaluation of attention and written-output concerns." },
                  { kind: "bullet_list", items: ["Review teacher observations", "Coordinate follow-up"] },
                  { kind: "table", rows: [["Domain", "Finding"], ["Attention", "Needs support"]], has_header: true, column_widths: [3500, 6500] },
                ],
              }],
            };
            reportWorkspace = {
              ...reportWorkspace,
              turns: [{
                id: "turn-1",
                model_id: args.modelId,
                timeline: [
                  { kind: "message", role: "user", text: args.instruction, created_at: new Date().toISOString() },
                  toolActivity("list_record_files", "list-1", "Listed 3 record files", {}, { file_count: 3, truncated: false }),
                  toolActivity("read_record_file", "read-1", "Read intake-parent-interview.txt, characters 0–3200", { filename: "intake-parent-interview.txt", offset: 0, limit: 8000 }, { filename: "intake-parent-interview.txt", offset: 0, returned_characters: 3200, total_characters: 3200, content_retained: false }),
                  toolActivity("propose_report_changes", "proposal-1", "Staged report changes for approval", { summary: "Create an initial evaluation report", operations: [{ kind: "set_title", title: proposedContent.title }, { kind: "add_section", position: 0, heading: proposedContent.sections[0].heading, blocks: proposedContent.sections[0].blocks }] }, { status: "pending_user_acceptance", proposal_id: "proposal-1", base_revision: reportWorkspace.draft.revision }),
                  { kind: "message", role: "assistant", text: "I staged a proposal for your review.", created_at: new Date().toISOString() },
                ],
                usage: { model_id: args.modelId, input_tokens: 1240, output_tokens: 220, cache_read_input_tokens: 0, cache_write_input_tokens: 0, cache_ttl: null, cost_usd: 0.00702, pricing_version: 4 },
                usage_complete: true,
                converse_calls: 3,
                tool_uses: 3,
                context_files: [],
                context_reads: [{
                  filename: "intake-parent-interview.txt",
                  offset: 0,
                  returned_characters: 3200,
                  total_characters: 3200,
                  read_at: new Date().toISOString(),
                }],
                created_at: new Date().toISOString(),
                completed_at: new Date().toISOString(),
              }],
              last_agent_revision: reportWorkspace.draft.revision,
              pending_proposal: {
                id: "proposal-1",
                report_id: reportWorkspace.report_id,
                base_revision: reportWorkspace.draft.revision,
                model_id: args.modelId,
                summary: "Create an initial evaluation report",
                operations: [
                  { kind: "set_title", title: proposedContent.title },
                  { kind: "add_section", position: 0, section: proposedContent.sections[0] },
                ],
                proposed_content: proposedContent,
                created_at: new Date().toISOString(),
              },
            };
            return {
              workspace: reportWorkspace,
              turn_id: "turn-1",
              attempt_id: "88888888-8888-4888-8888-888888888888",
              assistant_text: "I staged a proposal for your review.",
              usage: reportWorkspace.turns[0].usage,
              usage_complete: true,
              converse_calls: 3,
              tool_uses: 3,
              proposal_id: "proposal-1",
            };
          }
          if (cmd === "resolve_report_proposal") {
            window.__REPORT_COMMANDS__.push(cmd);
            window.__REPORT_INVOCATIONS__.push({ cmd, args: structuredClone(args) });
            const proposal = reportWorkspace.pending_proposal;
            if (args.reportId !== reportWorkspace.report_id || !proposal || args.proposalId !== proposal.id) throw "The pending proposal changed. Reload the report before continuing.";
            if (args.decision !== "accept" && args.decision !== "reject") throw "Invalid proposal decision.";
            if (args.decision === "accept") {
              reportWorkspace = {
                ...reportWorkspace,
                draft: {
                  ...reportWorkspace.draft,
                  revision: reportWorkspace.draft.revision + 1,
                  content: proposal.proposed_content,
                  updated_at: new Date().toISOString(),
                  last_applied_proposal_id: proposal.id,
                },
                pending_proposal: null,
                last_agent_revision: reportWorkspace.draft.revision + 1,
                template_import: reportWorkspace.template_import
                  ? { ...reportWorkspace.template_import, reviewed_revision: reportWorkspace.draft.revision + 1, review_required: false }
                  : null,
              };
              rememberReportDraft();
            } else {
              reportWorkspace = { ...reportWorkspace, pending_proposal: null };
            }
            return reportWorkspace;
          }
          if (cmd === "export_report_docx") {
            window.__REPORT_COMMANDS__.push(cmd);
            window.__REPORT_INVOCATIONS__.push({ cmd, args: structuredClone(args) });
            if (args.reportId !== reportWorkspace.report_id || args.expectedRevision !== reportWorkspace.draft.revision) throw "The report changed on another computer. Reload it before continuing.";
            const attemptedAt = new Date().toISOString();
            reportWorkspace = {
              ...reportWorkspace,
              last_export: {
                revision: args.expectedRevision,
                status: "exported",
                attempted_at: attemptedAt,
              },
            };
            return {
              exported: true,
              report_id: args.reportId,
              revision: args.expectedRevision,
              status: "exported",
              attempted_at: attemptedAt,
              status_persisted: true,
            };
          }

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

          // ── Local transcription ──────────────────────────────────────
          if (cmd === "get_local_transcription_status") {
            return {
              runtime_version: "0.2.0",
              accelerated: false,
              legacy_model_bytes: 0,
              settings: {
                settings_version: 1,
                speech_model: "whisper_small_q8",
                backend: "auto",
                gpu_device: 0,
                cpu_threads: 0,
                kv_precision: "auto",
                initial_prompt: "",
                condition_on_previous_text: true,
                max_previous_context_tokens: 223,
                temperature: 0,
                temperature_increment: 0.2,
                compression_ratio_threshold: 2.4,
                log_probability_threshold: -1,
                no_speech_threshold: 0.6,
                seed: 0,
              },
              models: [],
              backends: [{ backend: "auto", label: "Automatic", available: true }],
              devices: [],
            };
          }

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
