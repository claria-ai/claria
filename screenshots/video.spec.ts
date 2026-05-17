import { test, expect } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";
import * as fs from "fs";
import * as path from "path";

const BASE_URL = "http://localhost:1420";
const OUTPUT_DIR = "output/videos";

/** Pause so the viewer can see the current state. */
const pause = (ms = 1500) => new Promise((r) => setTimeout(r, ms));

test.beforeEach(async ({ page }) => {
  await page.addInitScript({ content: buildInitScript() });
});

test.afterEach(async ({}, testInfo) => {
  // Copy the recorded video to the output directory with a friendly name.
  const video = testInfo.attachments.find((a) => a.name === "video");
  if (video?.path) {
    fs.mkdirSync(OUTPUT_DIR, { recursive: true });
    const dest = path.join(OUTPUT_DIR, `${testInfo.title.replace(/\s+/g, "-")}.webm`);
    fs.copyFileSync(video.path, dest);
  }
});

test("client-record", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await pause(1000);

  // Navigate to clients list
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await pause(1500);

  // Open a client record
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=record]");
  await pause(2000);
});

test("ai-chat", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=chat]");
  await pause(800);

  // Switch to chat tab
  await page.click("[data-tab=chat]");
  const textarea = page.locator("textarea");
  await expect(textarea).toBeVisible();
  await pause(800);

  // Type a message character by character for demo effect
  const msg = "Please build a history for this client";
  for (const ch of msg) {
    await textarea.press(ch === " " ? "Space" : ch);
    await pause(40);
  }
  await pause(800);

  // Send and wait for response
  await page.click("text=Send");
  await page.waitForSelector("text=Referral");
  await pause(3000);
});

test("voice-memo", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=record]");
  await pause(800);

  // Start recording
  await page.click("text=Record Memo");
  await page.waitForSelector("text=Jane presented today", { timeout: 15000 });
  await pause(2500);

  // Stop and review
  await page.click("button:has-text('Done')");
  await page.waitForSelector("text=Review Memo");
  await pause(2500);
});

test("aws-infrastructure", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=provision]");
  await pause(800);

  // Navigate to AWS provisioning
  await page.click("[data-page=provision]");
  // Wait for the plan to load and show the in-sync state
  await page.waitForSelector("text=All resources are in sync", { timeout: 30000 });
  await pause(2500);
});

test("infra-chat", async ({ page }) => {
  // Navigate to provision first, then click Ask AI
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=provision]");
  await page.click("[data-page=provision]");
  await page.waitForSelector("text=All resources are in sync", { timeout: 30000 });
  await pause(500);

  // Click Ask AI to go to infra chat
  await page.click("text=Ask AI");
  const textarea = page.locator("textarea");
  await expect(textarea).toBeVisible();
  await pause(800);

  // Type question
  const msg = "Is my data encrypted and protected?";
  for (const ch of msg) {
    await textarea.press(ch === " " ? "Space" : ch);
    await pause(40);
  }
  await pause(800);

  await page.click("text=Send");
  await page.waitForSelector("text=well protected");
  await pause(3000);
});

test("preferences", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=preferences]");
  await pause(500);

  await page.click("[data-page=preferences]");
  await page.waitForSelector("text=Preferences");
  await pause(1000);

  // Expand sections one by one
  await page.click("summary:has-text('PDF Extraction Prompt')");
  await pause(1000);
  await page.click("summary:has-text('Memo Transcription')");
  await page.waitForSelector("text=Best Quality");
  await pause(1000);
  await page.click("summary:has-text('Preferred Model')");
  await page.waitForSelector("text=Claude Opus 4.6");
  await pause(1500);
});

test("file-history", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=record]");
  await page.waitForSelector("text=intake-parent-interview.txt");
  await pause(1000);

  // Enable version history
  await page.click('button[title="Show version history"]');
  await page.waitForSelector("text=No deleted files found.");
  await pause(1000);

  await page.locator('button[title="Version history"]').first().click();
  await page.waitForSelector("text=Version History:");
  await pause(1000);

  // Select and compare versions
  const checkboxes = page.locator('input[type="checkbox"]');
  await checkboxes.nth(0).check();
  await pause(500);
  await checkboxes.nth(1).check();
  await page.waitForSelector("text=2 versions selected");
  await pause(800);

  await page.click('button:has-text("Compare")');
  await page.waitForSelector("h4:has-text('Diff')");
  await pause(2500);
});
