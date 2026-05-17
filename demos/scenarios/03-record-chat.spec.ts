/**
 * Demo: Create a client record, add intake notes, and chat with AI.
 *
 * Starts from the Home Screen with existing config and clients.
 * Creates a new client, writes case notes, then asks Claude Opus
 * questions about the case and receives an analysis.
 */

import { test, expect } from "@playwright/test";
import { buildInitScript } from "../tauri-mock.js";
import { caseNotesText, chatQuestion } from "../fixtures.js";

const BASE_URL = "http://localhost:1420";

/** Type text character-by-character for a natural demo feel. */
async function typeSlowly(page: import("@playwright/test").Page, locator: import("@playwright/test").Locator, text: string, msPerChar = 80) {
  await locator.click();
  for (const char of text) {
    await locator.pressSequentially(char, { delay: 0 });
    await page.waitForTimeout(msPerChar);
  }
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript({
    content: buildInitScript({ hasConfig: true, scenario: "record-chat" }),
  });
});

test("create record and chat", async ({ page }) => {
  // ── Home Screen ──────────────────────────────────────────────────────
  await page.goto(BASE_URL);
  await page.waitForSelector("text=Client Files");
  await page.waitForTimeout(3000);

  await page.click("text=Client Files");

  // ── Client List ──────────────────────────────────────────────────────
  await page.waitForSelector("text=Clients");
  await page.waitForSelector("text=Jane Doe"); // Existing clients loaded
  await page.waitForTimeout(3000);

  // Create new client
  await page.click("button:has-text('New Client')");
  await page.waitForSelector("text=Create New Client");
  await page.waitForTimeout(2000);

  const nameInput = page.locator('input[placeholder="Client name"]');
  await typeSlowly(page, nameInput, "Alex Rivera");
  await page.waitForTimeout(2000);

  await page.click("button:has-text('Create'):not(:has-text('New'))");

  // ── Client Record View ───────────────────────────────────────────────
  await page.waitForSelector("[data-tab=record]", { timeout: 10000 });
  await page.waitForSelector("text=Alex Rivera");
  await page.waitForTimeout(4000); // Viewer sees empty record

  // Create a text file
  await page.click("button:has-text('Create Text File')");
  await page.waitForSelector("text=Create Text File");
  await page.waitForTimeout(2000);

  // Type filename — the modal has the placeholder "Filename (e.g. intake-notes)"
  const filenameInput = page.locator('input[placeholder*="Filename"]');
  await typeSlowly(page, filenameInput, "intake-notes");
  await page.waitForTimeout(1500);

  // Type case notes (use fill — typing 1500+ chars at 80ms would be too long)
  const contentArea = page.locator('textarea[placeholder="File content..."]');
  await contentArea.click();
  await contentArea.fill(caseNotesText);
  await page.waitForTimeout(3000); // Viewer reads the notes

  // Click the Create button inside the modal (not the "Create Text File" behind it)
  const modal = page.locator(".fixed.inset-0");
  await modal.locator("button:has-text('Create')").click();
  await page.waitForTimeout(3000); // File appears in the list

  // ── Switch to Chat Tab ───────────────────────────────────────────────
  await page.click("[data-tab=chat]");
  const textarea = page.locator("textarea");
  await expect(textarea).toBeVisible({ timeout: 10000 });
  await page.waitForTimeout(3000); // Viewer sees the chat interface

  // Type the question
  await typeSlowly(page, textarea, chatQuestion, 60);
  await page.waitForTimeout(3000); // Viewer reads the question

  // Send
  await page.click("button:has-text('Send')");

  // Wait for the AI response to render
  await page.waitForSelector("text=Primary Concerns", { timeout: 15000 });
  await page.waitForTimeout(6000); // Viewer reads the AI analysis

  // Scroll down to see more of the response
  const chatArea = page.locator(".overflow-y-auto").last();
  await chatArea.evaluate((el) => el.scrollTo({ top: el.scrollHeight, behavior: "smooth" }));
  await page.waitForTimeout(5000); // Viewer reads the rest
});
