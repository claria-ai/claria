import { defineConfig } from "@playwright/test";

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
    command: "npm run dev",
    cwd: "../claria-desktop-frontend",
    // Probe the entry module, not `/`. Vite answers `/` the moment it binds the
    // port, while the transform pipeline is still warming; `/src/main.tsx`
    // only answers once the module graph can actually be served.
    url: "http://localhost:1420/src/main.tsx",
    // Reusing whatever already holds :1420 is a local convenience — `cargo
    // tauri dev` serves the frontend on the same port. In CI it would mask a
    // stale or foreign server, so CI always starts its own.
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
    // Without these, a dev-server failure reaches the log as nothing at all.
    stdout: "pipe",
    stderr: "pipe",
  },
});
