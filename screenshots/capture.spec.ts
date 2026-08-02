import { test, expect } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";
import { driftedPlan } from "./fixtures.js";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";

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
  // Wait for the new Transcription section (rendered open by default) — its
  // presence confirms the cross-machine sync banner is also rendered above it.
  await page.waitForSelector("text=Default language");
  // Expand all the other collapsed sections so the full preferences surface
  // is visible in the screenshot.
  await page.click("summary:has-text('PDF Extraction Prompt')");
  await page.click("summary:has-text('Memo Transcription')");
  await page.waitForSelector("text=Best Quality");
  await page.click("summary:has-text('Preferred Model')");
  await page.waitForSelector("text=Claude Opus 4.6");
  await page.screenshot({ path: "output/preferences.png", fullPage: true });
});

test("transcribe wizard", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=record]");
  // Open the wizard via its dedicated button.
  await page.click("button:has-text('Upload Audio File')");
  await page.waitForSelector("text=Upload audio file");
  // The mocked `pick_audio_file` returns a path; clicking the picker button
  // populates the wizard's filePath state with that value.
  await page.click("button:has-text('Choose a file')");
  await page.waitForSelector("text=visit-2026-03-15.m4a");
  // Select Mixed (interpreter session) — the headline use case for the wizard.
  await page.click("text=Mixed (interpreter session)");
  // Enable translation so the screenshot shows the toggle in its "on" state.
  await page.click("text=Translate non-English segments to English");
  await page.waitForTimeout(300);
  await page.screenshot({ path: "output/transcribe-wizard.png", fullPage: false });
});

test("transcript editor", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=record]");
  await page.waitForSelector("text=session-2026-03-15.m4a");
  // The audio file is the last row in the seeded fixture (after .txt/.pdf),
  // so its preview-button is the last on the page.
  await page.locator('button[title="Preview text"]').last().click();
  // The Speakers pane only renders when the body has diarization headers.
  await page.waitForSelector("text=Speakers");
  await page.waitForTimeout(300);
  await page.screenshot({ path: "output/transcript-editor.png", fullPage: false });
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

test("client record settings", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.getByRole("button", { name: "Record settings" }).click();
  await page.waitForSelector("text=Record statistics");
  await page.waitForTimeout(300);
  await page.screenshot({ path: "output/client-record-settings.png", fullPage: true });
});

test("client writing", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=writing]");
  await page.click("[data-tab=writing]");
  await page.waitForSelector("[data-testid=report-proposal]");
  await page.waitForTimeout(300);
  await page.screenshot({ path: "output/client-writing.png", fullPage: true });
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
  await page.waitForSelector("[data-page=provision]");
  await page.click("[data-page=provision]");
  // Wait for the plan to load — one unified list, all resources visible
  await page.waitForSelector("text=all resources in sync");
  await page.screenshot({ path: "output/aws.png", fullPage: true });
});

test("aws drift", async ({ page }) => {
  // Override the plan fixture: this init script runs after the default
  // one from beforeEach, so its fixtures win.
  await page.addInitScript({ content: buildInitScript({ plan: driftedPlan }) });
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=provision]");
  await page.click("[data-page=provision]");
  // Drifted entries show their field diffs inline in the unified list; the
  // elevated-scope IAM policy adds the sync notice on top and the single
  // escalation CTA at the bottom.
  await page.waitForSelector("text=2 changes needed");
  await page.waitForSelector("text=Sync required");
  await page.waitForSelector("text=Provide Admin Credentials");
  await page.screenshot({ path: "output/aws-drift.png", fullPage: true });
});

test("infra chat", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=provision]");
  await page.click("[data-page=provision]");
  await page.waitForSelector("text=all resources in sync");
  await page.click("text=Ask AI");
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
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  // Resume the stored conversation — the chat history header (with the
  // account-spend link) only renders for resumed sessions.
  await page.click("text=Chat History");
  await page.waitForSelector('button[title="Resume conversation"]');
  await page.click('button[title="Resume conversation"]');
  await page.click("text=See account spend");
  // Wait for the chart to render — the "Total:" line appears once data loads
  await page.waitForSelector("text=Total:");
  await page.screenshot({ path: "output/cost-explorer.png", fullPage: true });
});

// ── File history screenshot ──────────────────────────────────────────

test("file history – diff", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=record]");
  await page.waitForSelector("text=intake-parent-interview.txt");
  // Enable version history mode and open version list
  await page.click('button[title="Show version history"]');
  await page.waitForSelector("text=No deleted files found.");
  await page.locator('button[title="Version history"]').first().click();
  await page.waitForSelector("text=Version History:");
  // Select two versions and compare
  const checkboxes = page.locator('input[type="checkbox"]');
  await checkboxes.nth(0).check();
  await checkboxes.nth(1).check();
  await page.waitForSelector("text=2 versions selected");
  await page.click('button:has-text("Compare")');
  await page.waitForSelector("h4:has-text('Diff')");
  await page.screenshot({ path: "output/history-diff.png", fullPage: true });
});
