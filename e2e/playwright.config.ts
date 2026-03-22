import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.ts",
  timeout: 120_000,
  workers: 1,
  use: {
    viewport: { width: 1024, height: 768 },
    deviceScaleFactor: 2,
    // Trace on first retry for debugging failures.
    trace: "on-first-retry",
  },
  webServer: {
    command: "npm run dev",
    cwd: "../claria-desktop-frontend",
    url: "http://localhost:1420",
    reuseExistingServer: true,
    timeout: 30_000,
  },
});
