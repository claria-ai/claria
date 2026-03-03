import { test, expect } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = "http://localhost:1420";

test.beforeEach(async ({ page }) => {
  // Inject Tauri IPC mock before the app loads
  await page.addInitScript({ content: buildInitScript() });
});

test("start screen", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.screenshot({ path: "output/start.png", fullPage: true });
});

test("about page", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=about]");
  await page.click("[data-page=about]");
  await page.waitForSelector("text=About Claria");
  await page.screenshot({ path: "output/about.png", fullPage: true });
});

test("preferences page", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=preferences]");
  await page.click("[data-page=preferences]");
  await page.waitForSelector("text=Preferences");
  // Expand all collapsed sections
  await page.click("summary:has-text('PDF Extraction Prompt')");
  await page.click("summary:has-text('Memo Transcription')");
  await page.waitForSelector("text=Best Quality");
  await page.click("summary:has-text('Preferred Model')");
  await page.waitForSelector("text=Claude Opus 4.6");
  await page.screenshot({ path: "output/preferences.png", fullPage: true });
});

test("client list", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.screenshot({ path: "output/clients.png", fullPage: true });
});

test("client record", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=record]");
  await page.screenshot({ path: "output/client-record.png", fullPage: true });
});

test("client chat", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=chat]");
  await page.click("[data-tab=chat]");
  const textarea = page.locator("textarea");
  await expect(textarea).toBeVisible();
  await textarea.fill("Please build a history for this client");
  await page.click("text=Send");
  // Wait for the assistant response to render
  await page.waitForSelector("text=Referral");
  await page.screenshot({ path: "output/client-chat.png", fullPage: true });
});

test("memo recording", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=record]");
  // Start recording — fake media stream provides audio data
  await page.click("text=Record Memo");
  // Wait for the first transcription cycle (~4s) to populate the live transcript
  await page.waitForSelector("text=Jane presented today", { timeout: 15000 });
  await page.screenshot({ path: "output/memo-recording.png", fullPage: true });
});

test("memo review", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=record]");
  // Start recording and wait for transcript
  await page.click("text=Record Memo");
  await page.waitForSelector("text=Jane presented today", { timeout: 15000 });
  // Click Done to trigger final transcription and open review modal
  await page.click("button:has-text('Done')");
  await page.waitForSelector("text=Review Memo");
  await page.screenshot({ path: "output/memo-review.png", fullPage: true });
});

test("aws management", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=aws]");
  await page.click("[data-page=aws]");
  // Wait for the plan to load and render the Ready section
  await page.waitForSelector("text=all resources in sync");
  // Expand the collapsed Ready section
  await page.click("summary:has-text('Ready')");
  await page.screenshot({ path: "output/aws.png", fullPage: true });
});

test("infra chat", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=aws]");
  await page.click("[data-page=aws]");
  await page.waitForSelector("[data-page=infra-chat]");
  await page.click("[data-page=infra-chat]");
  const textarea = page.locator("textarea");
  await expect(textarea).toBeVisible();
  await textarea.fill("Is my data encrypted and protected?");
  await page.click("text=Send");
  // Wait for the assistant response to render
  await page.waitForSelector("text=well protected");
  await page.screenshot({ path: "output/infra-chat.png", fullPage: true });
});

test("cost explorer", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=aws]");
  await page.click("[data-page=aws]");
  await page.waitForSelector("[data-page=cost-explorer]");
  await page.click("[data-page=cost-explorer]");
  // Wait for the chart to render — the "Total:" line appears once data loads
  await page.waitForSelector("text=Total:");
  await page.screenshot({ path: "output/cost-explorer.png", fullPage: true });
});
