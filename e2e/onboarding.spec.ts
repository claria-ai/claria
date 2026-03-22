/**
 * E2E test: Fresh AWS account → full onboarding → records view.
 *
 * This test walks through the complete new-user journey:
 *   1. Start screen (no config) → "Create New System"
 *   2. Guide: AWS Account → Next
 *   3. Guide: MFA Setup → Done — Next
 *   4. Guide: Access Key → Next
 *   5. Credential Intake → enter root creds → Check → Bootstrap
 *   6. Bootstrap completes → Continue to Provisioning
 *   7. Scan & Provision → Start Scan → review plan → Apply Changes
 *   8. Provisioning complete → Go to AWS (or navigate to start)
 *   9. Start screen now shows "Client Files" → navigate to clients
 *  10. Client list (empty) → create a new client → lands on records view
 */

import { test, expect } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = "http://localhost:1420";

test.beforeEach(async ({ page }) => {
  // Inject the stateful Tauri mock before the app loads
  await page.addInitScript({ content: buildInitScript() });
});

test("fresh account onboarding → bootstrap → provision → records", async ({ page }) => {
  // ── 1. Start screen (no config) ──────────────────────────────────────
  await page.goto(BASE_URL);
  await page.waitForSelector("text=Claria");

  // Should show "Create New System" since has_config returns false
  const createBtn = page.locator("text=Create New System");
  await expect(createBtn).toBeVisible();

  // Should NOT show "Client Files" (no config yet)
  await expect(page.locator("text=Client Files")).not.toBeVisible();

  await page.waitForTimeout(5000);
  await createBtn.click();

  // ── 2. Guide: AWS Account (Step 1) ───────────────────────────────────
  await page.waitForSelector("text=Step 1: Create an AWS Account");
  await expect(page.locator("text=Step 1")).toBeVisible();

  // Click Next
  await page.waitForTimeout(5000);
  await page.click("button:has-text('Next')");

  // ── 3. Guide: MFA Setup (Step 2) ────────────────────────────────────
  await page.waitForSelector("text=Step 2: Secure Your Root Account");
  await expect(page.locator("text=Highly recommended")).toBeVisible();

  // Click "Done — Next"
  await page.waitForTimeout(5000);
  await page.click("button:has-text('Done — Next')");

  // ── 4. Guide: Access Key (Step 3) ───────────────────────────────────
  await page.waitForSelector("text=Step 3: Create a Root Access Key");
  await expect(page.locator("text=Copy both")).toBeVisible();

  // Click Next
  await page.waitForTimeout(5000);
  await page.click("button:has-text('Next')");

  // ── 5. Credential Intake (Step 4) ───────────────────────────────────
  await page.waitForSelector("text=Step 4: Configure Credentials");

  // "I'm new to AWS" mode should be selected by default (inline)
  await expect(page.locator("button:has-text(\"I'm new to AWS\")")).toHaveClass(/bg-blue-500/);

  // Fill in root credentials
  const accessKeyInput = page.locator('input[placeholder="AKIAIOSFODNN7EXAMPLE"]');
  await accessKeyInput.fill("AKIAIOSFODNN7EXAMPLE");

  const secretKeyInput = page.locator('input[placeholder="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"]');
  await secretKeyInput.fill("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");

  // Click "Check Credentials"
  await page.waitForTimeout(5000);
  await page.click("button:has-text('Check Credentials')");

  // Wait for assessment result — should show Root Account
  await page.waitForSelector("text=Root Account");
  await expect(page.locator("text=Authenticated as the root user")).toBeVisible();

  // Should show the bootstrap notice
  await expect(page.locator("text=Root credentials detected")).toBeVisible();
  await expect(page.locator("text=dedicated IAM user")).toBeVisible();

  // Click "Set Up Secure User" to start bootstrap
  await page.waitForTimeout(5000);
  await page.click("button:has-text('Set Up Secure User')");

  // Wait for bootstrap to complete — all steps should show ✅
  await page.waitForSelector("text=Secure IAM user created");

  // Verify bootstrap steps rendered
  await expect(page.locator("text=Create IAM policy")).toBeVisible();
  await expect(page.locator("text=Create IAM user")).toBeVisible();
  await expect(page.locator("text=Create access key")).toBeVisible();

  // Click "Continue to Provisioning"
  await page.waitForTimeout(5000);
  await page.click("button:has-text('Continue to Provisioning')");

  // ── 6. Scan & Provision (Step 5) ────────────────────────────────────
  await page.waitForSelector("text=Step 5: Review & Provision");
  await expect(page.locator("text=read-only operation")).toBeVisible();

  // Click "Start Scan"
  await page.waitForTimeout(5000);
  await page.click("button:has-text('Start Scan')");

  // Wait for scan to complete and plan to appear
  await page.waitForSelector("text=Apply Changes", { timeout: 30_000 });

  // Verify some plan entries are visible
  await expect(page.locator("text=IAM User")).toBeVisible();
  await expect(page.getByText("S3 Bucket", { exact: true })).toBeVisible();
  await expect(page.getByText("CloudTrail Trail", { exact: true })).toBeVisible();

  // Click "Apply Changes"
  await page.waitForTimeout(5000);
  await page.click("button:has-text('Apply Changes')");

  // Wait for provisioning to complete
  await page.waitForSelector("text=Provisioning complete!", { timeout: 30_000 });

  // Click "Go to AWS" to navigate to the AWS management page
  // (or we could navigate to start — let's go to start to test the full loop)
  // After provisioning, the "Go to AWS" button should be visible
  // But we want to end on the records view, so let's navigate to start first
  // Actually, ScanProvision shows "Go to AWS" on done. Let's check what's there.
  // The ScanProvision page shows "Re-scan" and "Go to AWS" buttons after done.
  // We need to get to the start screen to see "Client Files".

  // After provisioning completes, "Go to AWS" is visible. Click it.
  await page.waitForTimeout(5000);
  await page.click("button:has-text('Go to AWS')");

  // ── 7. AWS Management page ─────────────────────────────────────────
  // The AWS page loads and runs a plan(). It should show all-in-sync.
  await page.waitForSelector("text=all resources in sync", { timeout: 30_000 });

  // Navigate back to start using the back button (left arrow)
  // The AwsManage page has a back button that goes to "start"
  await page.waitForTimeout(5000);
  await page.click("button:has-text('Back')", { timeout: 5_000 }).catch(async () => {
    // AwsManage uses a chevron SVG button for back, find it
    await page.locator("svg").first().click();
  });

  // Wait for start screen
  await page.waitForSelector("text=Claria", { timeout: 5_000 });

  // Config is now saved, so "Client Files" should appear
  const clientFilesBtn = page.locator("text=Client Files");
  await expect(clientFilesBtn).toBeVisible({ timeout: 5_000 });

  // "Create New System" should NOT be visible (config exists)
  await expect(page.locator("text=Create New System")).not.toBeVisible();

  // Click "Client Files"
  await page.waitForTimeout(5000);
  await clientFilesBtn.click();

  // ── 8. Client list (empty) ──────────────────────────────────────────
  await page.waitForSelector("text=Clients");
  await expect(page.locator("text=No client records yet")).toBeVisible();

  // Create a new client
  await page.waitForTimeout(5000);
  await page.click("button:has-text('New Client')");
  await page.waitForSelector("text=Create New Client");

  const nameInput = page.locator('input[placeholder="Client name"]');
  await nameInput.fill("Jane Doe");

  await page.waitForTimeout(5000);
  await page.click("button:has-text('Create')");

  // ── 9. Client record view ──────────────────────────────────────────
  // After creating a client, the app navigates to the client record view
  await page.waitForSelector("[data-tab=record]", { timeout: 10_000 });

  // Verify we're on the record view
  await expect(page.locator("text=Jane Doe")).toBeVisible();

  // The record tab should be visible and active
  await expect(page.locator("[data-tab=record]")).toBeVisible();
  await page.waitForTimeout(5000);
});
