import { expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ log: vi.fn(async () => undefined) }));

vi.mock("./bindings", () => ({
  commands: { logFrontendEvent: mocks.log },
}));

it("forwards webview console errors and uncaught errors to the backend", async () => {
  const originalError = console.error;
  const originalWarn = console.warn;
  console.error = vi.fn();
  console.warn = vi.fn();

  const { installGlobalErrorHandlers } = await import("./logBridge");
  installGlobalErrorHandlers();

  console.error("React event handler failed", new Error("button exploded"));
  window.dispatchEvent(
    new ErrorEvent("error", {
      message: "uncaught click failure",
      error: new Error("uncaught click failure"),
      filename: "app.js",
      lineno: 42,
      colno: 7,
    })
  );
  const rejection = new Event("unhandledrejection");
  Object.defineProperty(rejection, "reason", {
    value: new Error("background task failed"),
  });
  window.dispatchEvent(rejection);

  await vi.waitFor(() => {
    expect(mocks.log).toHaveBeenCalledWith(
      "error",
      expect.stringContaining("Webview console.error: React event handler failed")
    );
    expect(mocks.log).toHaveBeenCalledWith(
      "error",
      expect.stringContaining("Uncaught webview error: Error: uncaught click failure")
    );
    expect(mocks.log).toHaveBeenCalledWith(
      "error",
      expect.stringContaining(
        "Unhandled webview promise rejection: Error: background task failed"
      )
    );
  });

  console.error = originalError;
  console.warn = originalWarn;
});
