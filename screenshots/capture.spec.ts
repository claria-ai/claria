import { test, expect, type Page } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";
import { driftedPlan, freshWritingWorkspace } from "./fixtures.js";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";
const SCREENSHOT_TIME = new Date("2026-03-03T20:00:00Z");

test.beforeEach(async ({ page }) => {
  // Keep dates and generated filenames stable across local and CI captures.
  // Fixed time leaves real timers running, which the memo tests require.
  await page.clock.setFixedTime(SCREENSHOT_TIME);
  // Inject Tauri IPC mock before the app loads
  await page.addInitScript({ content: buildInitScript() });
});

async function capture(page: Page, filename: string, fullPage: boolean) {
  await page.screenshot({
    path: `output/${filename}`,
    fullPage,
    animations: "disabled",
  });
}

async function settleConversationAtStart(page: Page) {
  // Chat auto-scrolls smoothly after appending the response. Wait for that
  // motion to finish, then show the start of the exchange consistently.
  await page.waitForTimeout(500);
  const scroller = page.locator(
    "#chat-session-panel-conversation .overflow-y-auto"
  );
  await expect(scroller).toBeVisible();
  await scroller.evaluate((element) => {
    element.scrollTop = 0;
  });
  await page.waitForTimeout(50);
}

test("start screen", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await capture(page, "start.png", true);
});

test("about page", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=about]");
  await page.click("[data-page=about]");
  await page.waitForSelector("text=About Claria");
  await capture(page, "about.png", true);
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
  await page.waitForSelector("text=Memo speech model");
  await page.waitForSelector("text=Claude Opus 4.6");
  await page.click("summary:has-text('Writer Templates')");
  await page.waitForSelector("text=Comprehensive evaluation");
  // The saved-prompt library, one reusable instruction per workflow phase.
  await page.click("summary:has-text('Prompt Library')");
  await page.waitForSelector("text=Phase 1 — Referral, background & history");
  // Writer prompt editors with their read-only trust rules, and the model
  // tuning knobs.
  await page.click("summary:has-text('Writer Prompt')");
  await page.waitForSelector("text=Claria always appends these trust rules");
  await page.click("summary:has-text('Whole-Report Prompt')");
  await page.click("summary:has-text('Model Tuning')");
  await page.waitForSelector("text=Adaptive reasoning");
  await capture(page, "preferences.png", true);
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
  await capture(page, "transcribe-wizard.png", false);
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
  await capture(page, "transcript-editor.png", false);
});

test("client list", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await capture(page, "clients.png", true);
});

test("client record", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=record]");
  await capture(page, "client-record.png", true);
});

test("client record settings", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.getByRole("button", { name: "Record settings" }).click();
  await page.waitForSelector("text=Name history");
  await page.waitForTimeout(300);
  await capture(page, "client-record-settings.png", true);
});

test("client writing", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=writing]");
  await page.click("[data-tab=writing]");
  await page.getByRole("tab", { name: "Write with Claude" }).click();
  await page.waitForSelector("[data-testid=report-proposal]");
  await page.waitForTimeout(300);
  await capture(page, "client-writing.png", true);
});

test("client writing prompt library", async ({ page }) => {
  // A session before its first turn, so the setup pane offers whole-report
  // generation; the saved-prompt picker prefills the guidance box with the
  // first phase of a phased report workflow.
  await page.addInitScript({
    content: buildInitScript({
      load_report_workspace: freshWritingWorkspace,
      start_report_workspace: freshWritingWorkspace,
    }),
  });
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=clients]");
  await page.click("[data-page=clients]");
  await page.waitForSelector("[data-client]");
  await page.click("[data-client]:first-child");
  await page.waitForSelector("[data-tab=writing]");
  await page.click("[data-tab=writing]");
  await page.waitForSelector("text=Fill the whole report");
  await page.selectOption('select[aria-label="Insert saved prompt"]', {
    label: "Phase 1 — Referral, background & history",
  });
  const guidance = page.getByLabel("Full report guidance");
  await expect(guidance).toHaveValue(/Reason for Referral/);
  // Prefill focuses the textarea with the caret at the end; show the
  // instruction from its first line.
  await guidance.evaluate((element) => {
    const textarea = element as HTMLTextAreaElement;
    textarea.setSelectionRange(0, 0);
    textarea.scrollTop = 0;
    textarea.blur();
  });
  await page.waitForTimeout(300);
  await capture(page, "writing-prompt-library.png", true);
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
  // Wait for the complete assistant response, then reset smooth auto-scroll.
  await page.waitForSelector("text=Would you like me to draft");
  await settleConversationAtStart(page);
  await capture(page, "client-chat.png", true);
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
  await capture(page, "memo-recording.png", true);
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
  await capture(page, "memo-review.png", true);
});

test("aws management", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector("[data-page=provision]");
  await page.click("[data-page=provision]");
  // Wait for the plan to load — one unified list, all resources visible
  await page.waitForSelector("text=all resources in sync");
  await capture(page, "aws.png", true);
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
  await capture(page, "aws-drift.png", true);
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
  // Wait for the complete assistant response, then reset smooth auto-scroll.
  await page.waitForSelector("text=no drift detected");
  await settleConversationAtStart(page);
  await capture(page, "infra-chat.png", true);
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
  await page.getByRole("tab", { name: "Costs and cache" }).click();
  await page.click("text=See account spend");
  // Wait for the chart to render — the "Total:" line appears once data loads
  await page.waitForSelector("text=Total:");
  await capture(page, "cost-explorer.png", true);
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
  await capture(page, "history-diff.png", true);
});
