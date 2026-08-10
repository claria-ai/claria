/**
 * Frontend logging bridge.
 *
 * UI code is allowed to degrade gracefully (hide a badge, fall back to a
 * default) but never silently: every swallowed error is reported here so it
 * has somewhere to go.
 */

import { commands } from "./bindings";
import type { FrontendLogLevel } from "./bindings";

export type { FrontendLogLevel };

export function logFrontendEvent(
  level: FrontendLogLevel,
  message: string
): void {
  // The webview devtools console keeps a local copy for interactive debugging.
  console.error(`[claria:${level}] ${message}`);
  // Forward to the backend so the event lands in the console ring buffer and
  // the rolling log files. Fire-and-forget: if the bridge itself fails (e.g.
  // outside a Tauri window in tests), the console line above is the fallback
  // destination — reporting the failure here again would loop.
  void commands.logFrontendEvent(level, message).catch(() => {});
}

/** Report uncaught errors and unhandled rejections. Install once at startup. */
export function installGlobalErrorHandlers(): void {
  window.addEventListener("error", (event) => {
    logFrontendEvent(
      "error",
      `Uncaught error: ${event.message} (${event.filename}:${event.lineno})`
    );
  });
  window.addEventListener("unhandledrejection", (event) => {
    logFrontendEvent(
      "error",
      `Unhandled promise rejection: ${String(event.reason)}`
    );
  });
}
