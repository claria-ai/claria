/**
 * Demo: Bootstrap a fresh AWS account.
 *
 * Walks through the complete onboarding flow from a blank slate:
 * Home Screen → guides → credentials → bootstrap → scan → provision.
 */

import { test, expect } from "@playwright/test";
import { buildInitScript } from "../tauri-mock.js";

const BASE_URL = "http://localhost:1420";

/** Type text character-by-character for a natural demo feel. */
async function typeSlowly(page: import("@playwright/test").Page, selector: string, text: string, msPerChar = 80) {
  const el = page.locator(selector);
  await el.click();
  for (const char of text) {
    await el.pressSequentially(char, { delay: 0 });
    await page.waitForTimeout(msPerChar);
  }
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript({
    content: buildInitScript({ hasConfig: false, scenario: "bootstrap" }),
  });
});

test("bootstrap fresh account", async ({ page }) => {
  // ── Home Screen (no config) ──────────────────────────────────────────
  await page.goto(BASE_URL);
  await page.waitForSelector("text=Create New System");
  await page.waitForTimeout(4000); // Viewer sees the empty home screen

  await page.click("text=Create New System");

  // ── Step 1: AWS Account Guide ────────────────────────────────────────
  await page.waitForSelector("text=Step 1: Create an AWS Account");
  await page.waitForTimeout(5000); // Viewer reads guide content

  await page.click("button:has-text('Next')");

  // ── Step 2: MFA Setup Guide ──────────────────────────────────────────
  await page.waitForSelector("text=Step 2: Secure Your Root Account");
  await page.waitForTimeout(5000);

  await page.click("button:has-text('Done — Next')");

  // ── Step 3: Access Key Guide ─────────────────────────────────────────
  await page.waitForSelector("text=Step 3: Create a Root Access Key");
  await page.waitForTimeout(5000);

  await page.click("button:has-text('Next')");

  // ── Step 4: Credential Intake ────────────────────────────────────────
  await page.waitForSelector("text=Step 4: Configure Credentials");
  await page.waitForTimeout(3000); // Viewer sees the form

  // Type access key slowly
  await typeSlowly(page, 'input[placeholder="AKIAIOSFODNN7EXAMPLE"]', "AKIAIOSFODNN7EXAMPLE");
  await page.waitForTimeout(2000);

  // Type secret key slowly
  await typeSlowly(
    page,
    'input[placeholder="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"]',
    "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
  );
  await page.waitForTimeout(2000);

  // Check credentials
  await page.click("button:has-text('Check Credentials')");
  await page.waitForSelector("text=Root Account");
  await page.waitForTimeout(5000); // Viewer reads the assessment

  // Bootstrap
  await page.click("button:has-text('Set Up Secure User')");
  await page.waitForSelector("text=Secure IAM user created", { timeout: 15000 });
  await page.waitForTimeout(4000); // Viewer reads bootstrap results

  // Continue to provisioning
  await page.click("button:has-text('Continue to Provisioning')");

  // ── Step 5: Scan & Provision ─────────────────────────────────────────
  await page.waitForSelector("text=Step 5: Review & Provision");
  await page.waitForTimeout(3000);

  await page.click("button:has-text('Start Scan')");
  await page.waitForSelector("text=Apply Changes", { timeout: 30000 });
  await page.waitForTimeout(6000); // Viewer reviews the plan

  await page.click("button:has-text('Apply Changes')");
  await page.waitForSelector("text=Provisioning complete!", { timeout: 30000 });
  await page.waitForTimeout(5000); // Final state — viewer sees success
});
