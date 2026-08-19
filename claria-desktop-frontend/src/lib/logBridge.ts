/**
 * Frontend logging bridge.
 *
 * UI code is allowed to degrade gracefully (hide a badge, fall back to a
 * default) but never silently: swallowed errors, uncaught failures, and the
 * webview's warning/error console are forwarded to the Claria Console.
 */

import { commands } from "./bindings";
import type { FrontendLogLevel } from "./bindings";

export type { FrontendLogLevel };

// Keep unwrapped sinks so forwarding a console error never recursively creates
// another console error. These still preserve the webview devtools output.
const localConsoleError = console.error.bind(console);
const localConsoleWarn = console.warn.bind(console);
const localConsoleInfo = console.info.bind(console);
let handlersInstalled = false;

export function logFrontendEvent(
  level: FrontendLogLevel,
  message: string
): void {
  const localMessage = `[claria:${level}] ${message}`;
  if (level === "error") localConsoleError(localMessage);
  else if (level === "warn") localConsoleWarn(localMessage);
  else localConsoleInfo(localMessage);

  forwardToBackend(level, message);
}

/** Install uncaught-error and webview-console forwarding once at startup. */
export function installGlobalErrorHandlers(): void {
  if (handlersInstalled) return;
  handlersInstalled = true;

  const webviewConsoleError = console.error.bind(console);
  const webviewConsoleWarn = console.warn.bind(console);
  console.error = (...values: unknown[]) => {
    webviewConsoleError(...values);
    forwardConsole("error", values);
  };
  console.warn = (...values: unknown[]) => {
    webviewConsoleWarn(...values);
    forwardConsole("warn", values);
  };

  window.addEventListener("error", (event) => {
    const location = event.filename
      ? ` (${event.filename}:${event.lineno}:${event.colno})`
      : "";
    logFrontendEvent(
      "error",
      `Uncaught webview error: ${describeConsoleValue(event.error ?? event.message)}${location}`
    );
  });
  window.addEventListener("unhandledrejection", (event) => {
    logFrontendEvent(
      "error",
      `Unhandled webview promise rejection: ${describeConsoleValue(event.reason)}`
    );
  });
}

function forwardConsole(
  level: Extract<FrontendLogLevel, "error" | "warn">,
  values: unknown[]
): void {
  const message = values.map(describeConsoleValue).join(" ").trim();
  if (message.startsWith("[claria:")) return;
  forwardToBackend(
    level,
    `Webview console.${level}: ${message || "(no message)"}`
  );
}

function forwardToBackend(level: FrontendLogLevel, message: string): void {
  // Fire-and-forget: if the bridge itself fails (for example in browser-only
  // tests or against an older desktop binary), reporting that failure through
  // the same bridge would loop.
  try {
    void commands.logFrontendEvent(level, message).catch(() => {});
  } catch {
    // A synchronous bridge setup failure has only the original webview line.
  }
}

/**
 * Preserve useful Error stacks and primitive diagnostics without serializing
 * arbitrary objects that could contain client or document data.
 */
function describeConsoleValue(value: unknown): string {
  if (value instanceof Error) return value.stack || `${value.name}: ${value.message}`;
  if (typeof value === "string") return value;
  if (
    typeof value === "number" ||
    typeof value === "boolean" ||
    typeof value === "bigint"
  ) {
    return String(value);
  }
  if (value === null) return "null";
  if (value === undefined) return "undefined";
  return Object.prototype.toString.call(value);
}
