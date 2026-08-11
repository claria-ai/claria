// Build the init script string that mocks window.__TAURI_INTERNALS__
// so the app thinks it's running inside a Tauri webview.

import { fixtures } from "./fixtures.js";

export function buildInitScript(overrides: Record<string, unknown> = {}): string {
  const fixturesJson = JSON.stringify({ ...fixtures, ...overrides });

  // This script runs in the browser context before the app loads.
  // It sets up the Tauri internals mock so that invoke() calls
  // resolve with fixture data instead of hitting the Rust backend.
  return `
    window.__TAURI_INTERNALS__ = {
      metadata: {
        currentWindow: { label: "main" },
        currentWebview: { label: "main" },
      },
      // Channels use transformCallback to register message handlers.
      // We just assign an incrementing id — the callback is never invoked.
      _cbCounter: 0,
      transformCallback: function(cb, once) {
        var id = ++window.__TAURI_INTERNALS__._cbCounter;
        window["_" + id] = cb;
        return id;
      },
      invoke: async function(cmd, args) {
        const fixtures = ${fixturesJson};
        if (cmd === "log_frontend_event") return null;
        if (cmd === "plugin:app|version") {
          return "0.11.0";
        }
        if (cmd === "plugin:app|name") {
          return "Claria";
        }
        if (cmd === "plugin:app|tauri_version") {
          return "2.0.0";
        }
        if (cmd === "plugin:event|listen") {
          // Return a dummy unlisten id — drag-drop events etc.
          return 0;
        }
        if (cmd === "plugin:event|unlisten") {
          return;
        }
        if (cmd === "plugin:webview|get_all_webviews") {
          return [{ label: "main", url: "http://localhost:1420" }];
        }
        // Route commands that need arg-based dispatch
        if (cmd === "get_prompt" && args?.promptName) {
          const key = cmd + ":" + args.promptName;
          if (key in fixtures) return fixtures[key];
        }
        if (cmd === "get_file_version_text" && args?.versionId) {
          const key = cmd + ":" + args.versionId;
          if (key in fixtures) return fixtures[key];
        }
        if (cmd === "get_record_file_text" && args?.filename) {
          const key = cmd + ":" + args.filename;
          if (key in fixtures) return fixtures[key];
        }
        if (cmd in fixtures) {
          return fixtures[cmd];
        }
        console.warn("[tauri-mock] unhandled command:", cmd, args);
        return null;
      },
      convertFileSrc: function(path) {
        return path;
      },
    };
  `;
}
