/**
 * E2E test: Fresh AWS account → full onboarding → records view.
 *
 * Walks the complete new-user journey:
 *   1. Start screen (no config) → "Create New System"
 *   2. Guides: AWS Account → MFA Setup → Access Key
 *   3. Provision: enter root creds → Scan Resources → review plan → Bootstrap & Apply
 *   4. Provision done → Continue to Claria
 *   5. Start screen now shows "Client Files"
 *   6. Client list (empty) → create a new client → lands on record view
 */

import { test, expect } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";

test.beforeEach(async ({ page }) => {
  // Inject the stateful Tauri mock before the app loads
  await page.addInitScript({ content: buildInitScript() });
});

test("fresh account onboarding → provision → records", async ({ page }) => {
  // ── 1. Start screen (no config) ──────────────────────────────────────
  await page.goto(BASE_URL);
  await expect(page.getByRole("heading", { name: "Claria" })).toBeVisible();

  // Should show "Create New System" since has_config returns false
  const createBtn = page.getByRole("button", { name: "Create New System" });
  await expect(createBtn).toBeVisible();

  // Should NOT show "Client Files" (no config yet)
  await expect(page.getByText("Client Files")).not.toBeVisible();

  await createBtn.click();

  // ── 2. Guide: AWS Account (Step 1) ───────────────────────────────────
  await expect(
    page.getByRole("heading", { name: "Step 1: Create an AWS Account" })
  ).toBeVisible();
  await page.getByRole("button", { name: "Next" }).click();

  // ── 3. Guide: MFA Setup (Step 2) ─────────────────────────────────────
  await expect(
    page.getByRole("heading", { name: "Step 2: Secure Your Root Account with MFA" })
  ).toBeVisible();
  await page.getByRole("button", { name: "Done — Next" }).click();

  // ── 4. Guide: Access Key (Step 3) ────────────────────────────────────
  await expect(
    page.getByRole("heading", { name: "Step 3: Create a Root Access Key" })
  ).toBeVisible();
  await page.getByRole("button", { name: "Next" }).click();

  // ── 5. Provision: credential input (first run) ───────────────────────
  await expect(
    page.getByRole("heading", { name: "AWS Infrastructure" })
  ).toBeVisible();

  // Scan is disabled until both credential fields are filled
  const scanBtn = page.getByRole("button", { name: "Scan Resources" });
  await expect(scanBtn).toBeDisabled();

  await page.getByPlaceholder("Access Key ID").fill("AKIAIOSFODNN7EXAMPLE");
  await page
    .getByPlaceholder("Secret Access Key")
    .fill("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");

  await expect(scanBtn).toBeEnabled();
  await scanBtn.click();

  // ── 6. Plan review ───────────────────────────────────────────────────
  await expect(page.getByText("changes needed")).toBeVisible({ timeout: 30_000 });

  // Some representative plan entries
  await expect(page.getByText("IAM User", { exact: true })).toBeVisible();
  await expect(page.getByText("S3 Bucket", { exact: true })).toBeVisible();
  await expect(page.getByText("CloudTrail Trail", { exact: true })).toBeVisible();

  // Fresh account needs elevated resources created → bootstrap path
  await page.getByRole("button", { name: "Bootstrap & Apply" }).click();

  // ── 7. Apply completes ───────────────────────────────────────────────
  await expect(
    page.getByText("All resources provisioned successfully.")
  ).toBeVisible({ timeout: 30_000 });
  await expect(page.getByText("all resources in sync")).toBeVisible();

  await page.getByRole("button", { name: "Continue to Claria" }).click();

  // ── 8. Start screen with config ──────────────────────────────────────
  // Config is now saved, so "Client Files" appears and the wizard entry is gone
  const clientFilesBtn = page.getByRole("button", { name: "Client Files" });
  await expect(clientFilesBtn).toBeVisible();
  await expect(page.getByText("Create New System")).not.toBeVisible();

  await clientFilesBtn.click();

  // ── 9. Client list (empty) → create a client ─────────────────────────
  await expect(page.getByText("No client records yet")).toBeVisible();

  await page.getByRole("button", { name: "New Client" }).click();
  await expect(page.getByText("Create New Client")).toBeVisible();

  await page.getByPlaceholder("Client name").fill("Jane Doe");
  await page.getByRole("button", { name: "Create", exact: true }).click();

  // ── 10. Client record view ───────────────────────────────────────────
  await expect(page.locator("[data-tab=record]")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText("Jane Doe")).toBeVisible();
});
