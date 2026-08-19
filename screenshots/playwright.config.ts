import { defineConfig } from "@playwright/test";

// One knob for both halves of the run: the spec navigates here, and the dev
// server below is started on this URL's port. Overriding it is how a capture
// runs beside a `cargo tauri dev` session, which owns :1420 while it lives.
const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";
const PORT = new URL(BASE_URL).port || "1420";

export default defineConfig({
  testDir: ".",
  testMatch: "capture.spec.ts",
  timeout: 120_000,
  workers: 1,
  // Backstop only — the cold-start reload this used to paper over is fixed in
  // the frontend's `optimizeDeps`. A release must not fail on a lone flake.
  retries: process.env.CI ? 2 : 0,
  use: {
    viewport: { width: 1024, height: 768 },
    deviceScaleFactor: 2,
    timezoneId: "America/Los_Angeles",
    permissions: ["microphone"],
    launchOptions: {
      args: [
        "--use-fake-device-for-media-stream",
        "--use-fake-ui-for-media-stream",
      ],
    },
  },
  webServer: {
    command: `npm run dev -- --port ${PORT} --strictPort`,
    cwd: "../claria-desktop-frontend",
    // Probe the entry module, not `/`. Vite answers `/` the moment it binds the
    // port, while the transform pipeline is still warming; `/src/main.tsx`
    // only answers once the module graph can actually be served.
    url: `${BASE_URL}/src/main.tsx`,
    // Reusing whatever already holds the port is a local convenience — `cargo
    // tauri dev` serves the frontend on :1420 too. In CI it would mask a
    // stale or foreign server, so CI always starts its own.
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
    // Without these, a dev-server failure reaches the log as nothing at all.
    stdout: "pipe",
    stderr: "pipe",
  },
});
