import { defineConfig } from "@playwright/test";

const testUrl = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";
const testPort = new URL(testUrl).port || "1420";

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
    video: "retain-on-failure",
  },
  webServer: {
    command: `npm run dev -- --port ${testPort}`,
    cwd: "../claria-desktop-frontend",
    url: testUrl,
    reuseExistingServer: true,
    timeout: 30_000,
  },
});
