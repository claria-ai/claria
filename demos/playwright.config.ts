import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "scenarios",
  testMatch: "*.spec.ts",
  timeout: 300_000,
  workers: 1,
  use: {
    viewport: { width: 1024, height: 768 },
    deviceScaleFactor: 2,
    video: {
      mode: "on",
      size: { width: 1024, height: 768 },
    },
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
    url: "http://localhost:1420",
    reuseExistingServer: true,
    timeout: 30_000,
  },
  outputDir: "output/test-results",
});
