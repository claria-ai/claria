/**
 * Demo: Synchronize cloud resources after a Claria policy update.
 *
 * Starts from the Home Screen with an existing config. Navigates to
 * AWS Management where 3 resources show drift from a policy update
 * (manifest_changed). Reviews the changes and applies them.
 */

import { test } from "@playwright/test";
import { buildInitScript } from "../tauri-mock.js";

const BASE_URL = "http://localhost:1420";

test.beforeEach(async ({ page }) => {
  await page.addInitScript({
    content: buildInitScript({ hasConfig: true, scenario: "sync" }),
  });
});

test("sync cloud resources after policy update", async ({ page }) => {
  // ── Home Screen (config exists) ──────────────────────────────────────
  await page.goto(BASE_URL);
  await page.waitForSelector("text=Client Files");
  await page.waitForTimeout(3000); // Viewer sees configured home screen

  // Navigate to AWS Management via the gear icon
  await page.click('[data-page="aws"]');

  // ── AWS Management — auto-scan shows drift ───────────────────────────
  // The page auto-scans on load. Wait for the plan to render.
  await page.waitForSelector("text=changes needed", { timeout: 30000 });
  await page.waitForTimeout(5000); // Viewer sees drift summary

  // The drifted resources should be visible in the "Changes" section.
  // Look for the drift labels
  await page.waitForSelector("text=New in this Claria update");
  await page.waitForTimeout(3000);

  // Expand the first drifted resource (S3 Bucket Versioning) to see details
  // The PlanEntryCard with actual data renders as <details>
  const versioningCard = page.locator("text=S3 Bucket Versioning").first();
  await versioningCard.click();
  await page.waitForTimeout(4000); // Viewer reads MFA Delete drift

  // Expand encryption card
  const encryptionCard = page.locator("text=S3 Bucket Encryption").first();
  await encryptionCard.click();
  await page.waitForTimeout(4000); // Viewer reads encryption upgrade details

  // Expand CloudTrail events card
  const cloudtrailCard = page.locator("text=CloudTrail S3 Events").first();
  await cloudtrailCard.click();
  await page.waitForTimeout(4000); // Viewer reads management events drift

  // Apply Changes
  await page.click("button:has-text('Apply Changes')");

  // Wait for apply to complete — the plan re-renders with all ok
  await page.waitForSelector("text=Changes applied successfully", { timeout: 30000 });
  await page.waitForTimeout(3000);

  // The plan should now show all in sync
  await page.waitForSelector("text=all resources in sync");
  await page.waitForTimeout(5000); // Final state
});
