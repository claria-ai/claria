/**
 * Frontend logging bridge.
 *
 * UI code is allowed to degrade gracefully (hide a badge, fall back to a
 * default) but never silently: every swallowed error is reported here so it
 * has somewhere to go.
 */

export type FrontendLogLevel = "error" | "warn" | "info";

export function logFrontendEvent(
  level: FrontendLogLevel,
  message: string
): void {
  // TODO(#73 LOG-3): forward through a `log_frontend_event` Tauri command so
  // frontend failures land in the backend console buffer and saved logs.
  // Until that command exists, the webview console is the destination.
  console.error(`[claria:${level}] ${message}`);
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
