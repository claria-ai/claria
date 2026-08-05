/**
 * Stateful Tauri IPC mock for demo video recording.
 *
 * Each scenario configures the mock with its initial state via the
 * `ScenarioConfig` parameter. The mock maintains mutable state so that
 * actions like "bootstrap → save config → scan → apply" flow naturally.
 */

import {
  savedConfig,
  chatModels,
  freshPlanEntries,
  allOkEntries,
  driftPlanEntries,
  existingClients,
  caseNotesText,
  chatQuestion,
  chatResponse,
} from "./fixtures.js";

export type ScenarioConfig = {
  /** Whether config exists at startup */
  hasConfig: boolean;
  /** Which scenario drives the plan/apply cycle */
  scenario: "bootstrap" | "sync" | "record-chat";
};

export function buildInitScript(config: ScenarioConfig): string {
  const configJson = JSON.stringify(savedConfig);
  const chatModelsJson = JSON.stringify(chatModels);
  const freshPlanJson = JSON.stringify(freshPlanEntries);
  const allOkJson = JSON.stringify(allOkEntries);
  const driftPlanJson = JSON.stringify(driftPlanEntries);
  const existingClientsJson = JSON.stringify(existingClients);
  const caseNotesJson = JSON.stringify(caseNotesText);
  const chatResponseJson = JSON.stringify(chatResponse);

  return `
    (() => {
      // ── Initial state from scenario config ───────────────────────────
      let configSaved = ${config.hasConfig};
      const scenario = "${config.scenario}";
      let appliedOnce = false;

      // ── Client state ─────────────────────────────────────────────────
      let clients = scenario === "record-chat" ? ${existingClientsJson} : [];
      let clientFiles = {};  // clientId → RecordFile[]
      let clientFileContents = {};  // clientId:filename → text

      // ── transformCallback support for Tauri Channel ──────────────────
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
          // ── Tauri plugin stubs ─────────────────────────────────────
          if (cmd === "plugin:app|version") return "0.11.0";
          if (cmd === "plugin:app|name") return "Claria";
          if (cmd === "plugin:app|tauri_version") return "2.0.0";
          if (cmd === "plugin:event|listen") return 0;
          if (cmd === "plugin:event|unlisten") return;
          if (cmd === "plugin:webview|get_all_webviews") {
            return [{ label: "main", url: "http://localhost:1420" }];
          }

          // ── Config ─────────────────────────────────────────────────
          if (cmd === "has_config") return configSaved;

          if (cmd === "load_config") {
            if (!configSaved) throw "No config found";
            return ${configJson};
          }

          if (cmd === "save_config") {
            configSaved = true;
            return null;
          }

          if (cmd === "delete_config") {
            configSaved = false;
            return null;
          }

          // ── Credential assessment ──────────────────────────────────
          if (cmd === "assess_credentials") {
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

          // ── Bootstrap IAM user ─────────────────────────────────────
          if (cmd === "bootstrap_iam_user") {
            const steps = [
              { name: "create_policy", status: "succeeded", detail: "arn:aws:iam::185735714230:policy/ClariaProvisionerAccess" },
              { name: "create_user", status: "succeeded", detail: "claria-admin" },
              { name: "attach_policy", status: "succeeded", detail: null },
              { name: "create_access_key", status: "succeeded", detail: "AKIA...0001" },
              { name: "validate_new_credentials", status: "succeeded", detail: null },
              { name: "delete_source_key", status: "succeeded", detail: "Root access key deleted" },
              { name: "write_config", status: "succeeded", detail: null },
              { name: "accept_model_agreements", status: "succeeded", detail: "3 models" },
            ];
            configSaved = true;
            return {
              success: true,
              steps,
              account_id: "185735714230",
              new_credentials: {
                access_key_id: "AKIAMOCKKEY00000001",
                secret_access_key: "mock-secret-key-00000001",
              },
              error: null,
            };
          }

          // ── Plan ───────────────────────────────────────────────────
          if (cmd === "plan") {
            if (appliedOnce) return ${allOkJson};
            if (scenario === "bootstrap") return ${freshPlanJson};
            if (scenario === "sync") return ${driftPlanJson};
            return ${allOkJson};
          }

          // ── Apply ──────────────────────────────────────────────────
          if (cmd === "apply") {
            appliedOnce = true;
            return ${allOkJson};
          }

          // ── AWS profiles ───────────────────────────────────────────
          if (cmd === "list_aws_profiles") return [];

          // ── Chat models ────────────────────────────────────────────
          if (cmd === "list_chat_models") return ${chatModelsJson};

          // ── Client operations ──────────────────────────────────────
          if (cmd === "list_clients") return clients;

          if (cmd === "create_client") {
            const newClient = {
              id: "new-client-" + Date.now(),
              name: args.name,
              created_at: new Date().toISOString(),
            };
            clients = [...clients, newClient];
            clientFiles[newClient.id] = [];
            return newClient;
          }

          // ── Record files ───────────────────────────────────────────
          if (cmd === "list_record_files") {
            return clientFiles[args.clientId] || [];
          }

          if (cmd === "list_editor_history") return [];

          if (cmd === "create_text_record_file") {
            const filename = args.filename.endsWith(".txt") ? args.filename : args.filename + ".txt";
            const file = {
              filename,
              size: (args.content || "").length,
              uploaded_at: new Date().toISOString(),
            };
            if (!clientFiles[args.clientId]) clientFiles[args.clientId] = [];
            clientFiles[args.clientId].push(file);
            clientFileContents[args.clientId + ":" + filename] = args.content || "";
            return null;
          }

          if (cmd === "list_record_context") {
            const files = clientFiles[args.clientId] || [];
            return files.map(f => ({
              filename: f.filename,
              text: clientFileContents[args.clientId + ":" + f.filename] || "",
            }));
          }

          // ── Chat message ───────────────────────────────────────────
          if (cmd === "chat_message") {
            return {
              chat_id: "demo-chat-" + Date.now(),
              content: ${chatResponseJson},
            };
          }

          // ── Prompts ────────────────────────────────────────────────
          if (cmd === "get_prompt") {
            if (args?.promptName === "system-prompt") {
              return "You are a clinical assistant helping a psychologist. Help gather relevant intake information. Be professional, empathetic, and concise.";
            }
            if (args?.promptName === "pdf-extraction") {
              return "Extract the complete text content from this document.";
            }
            return "";
          }

          if (cmd === "list_prompt_versions") return [];

          // ── Update check ───────────────────────────────────────────
          if (cmd === "check_for_updates") {
            return {
              current_version: "0.11.0",
              latest_version: "0.11.0",
              update_available: false,
              release_url: "",
            };
          }

          // ── Misc stubs ─────────────────────────────────────────────
          if (cmd === "reset_provisioner_state") return null;
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
          if (cmd === "list_deleted_clients") return [];
          if (cmd === "list_deleted_files") return [];
          if (cmd === "list_file_versions") return [];
          if (cmd === "probe_cost_explorer") return false;
          if (cmd === "count_client_context_tokens") return 1850;
          if (cmd === "count_infra_context_tokens") return 8530;

          console.warn("[tauri-mock-demo] unhandled command:", cmd, args);
          return null;
        },

        convertFileSrc: function(path) {
          return path;
        },
      };
    })();
  `;
}
