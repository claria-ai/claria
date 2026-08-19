import { Component, type ErrorInfo, type ReactNode } from "react";
import { logFrontendEvent } from "../lib/logBridge";

/**
 * Top-level render-crash containment: instead of a blank webview, show the
 * error with a reload button, and report the crash through the log bridge.
 */
export default class ErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    const stack = error.stack ?? String(error);
    logFrontendEvent(
      "error",
      `React render crash: ${stack}${info.componentStack ?? ""}`
    );
  }

  render() {
    if (this.state.error) {
      return (
        <div className="min-h-screen flex items-center justify-center bg-gray-50 p-8">
          <div className="max-w-lg bg-white border border-red-200 rounded-lg p-6 text-center">
            <h2 className="text-lg font-semibold text-gray-900">
              Claria hit an unexpected error
            </h2>
            <p className="mt-2 text-sm text-red-700 break-words">
              {String(this.state.error)}
            </p>
            <p className="mt-2 text-xs text-gray-500">
              Your data is stored in AWS and is not affected. Reloading
              restarts the interface.
            </p>
            <button
              type="button"
              onClick={() => window.location.reload()}
              className="mt-4 px-4 py-2 text-sm text-white bg-blue-600 rounded-lg hover:bg-blue-700"
            >
              Reload Claria
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}
