import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  testMatch: "video.spec.ts",
  timeout: 120_000,
  workers: 1,
  use: {
    viewport: { width: 1024, height: 768 },
    deviceScaleFactor: 2,
    permissions: ["microphone"],
    launchOptions: {
      args: [
        "--use-fake-device-for-media-stream",
        "--use-fake-ui-for-media-stream",
      ],
    },
    video: {
      mode: "on",
      size: { width: 2048, height: 1536 },
    },
  },
  webServer: {
    command: "npm run dev",
    cwd: "../claria-desktop-frontend",
    // Same readiness and reuse rules as playwright.config.ts.
    url: "http://localhost:1420/src/main.tsx",
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
