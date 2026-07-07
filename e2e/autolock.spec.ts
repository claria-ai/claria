/**
 * E2E test: auto-lock setup → manual lock → PIN unlock.
 *
 *   1. Start with a pre-configured system → Preferences → Security
 *   2. Set up auto-lock with a PIN
 *   3. Lock now → opaque lock screen hides the app
 *   4. Wrong PIN rejected, correct PIN unlocks
 */

import { test, expect } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = "http://localhost:1420";

const PIN = "123456";

test.beforeEach(async ({ page }) => {
  await page.addInitScript({ content: buildInitScript({ preConfigured: true }) });
});

test("auto-lock setup, manual lock, and PIN unlock", async ({ page }) => {
  // ── 1. Preferences → Security section ────────────────────────────────
  await page.goto(BASE_URL);
  await page.waitForSelector("text=Claria");

  await page.click('[data-page="preferences"]');
  await page.waitForSelector("text=Preferences");

  await page.click("summary:has-text('Security')");
  await expect(page.locator("text=Auto-lock hides Claria")).toBeVisible();

  // ── 2. Enable auto-lock with a PIN ───────────────────────────────────
  await page.click("button:has-text('Set up auto-lock')");
  await page.getByPlaceholder(/Choose a PIN/).fill(PIN);
  await page.getByPlaceholder(/Repeat PIN/).fill(PIN);
  await page.click("button:has-text('Enable auto-lock')");

  // Enabled view shows the management buttons and the "on" badge.
  await expect(page.locator("button:has-text('Lock now')")).toBeVisible();
  await expect(page.locator("text=Auto-lock on")).toBeVisible();

  // ── 3. Manual lock hides the app ─────────────────────────────────────
  await page.click("button:has-text('Lock now')");
  await expect(page.locator("text=Claria is locked")).toBeVisible();

  // The lock overlay is opaque and PHI-free. The page behind it stays
  // mounted (drafts survive), so check occlusion, not DOM visibility:
  // every sampled point must hit the overlay, not the content behind it.
  const covered = await page.evaluate(() => {
    const points: [number, number][] = [
      [window.innerWidth / 2, window.innerHeight / 2],
      [10, 10],
      [window.innerWidth - 10, window.innerHeight - 10],
    ];
    return points.every((p) => {
      const el = document.elementFromPoint(p[0], p[1]);
      return el !== null && el.closest(".fixed.inset-0") !== null;
    });
  });
  expect(covered).toBe(true);

  // ── 4. Wrong PIN rejected, correct PIN unlocks ───────────────────────
  await page.getByLabel("PIN").fill("999999");
  await page.click("button:has-text('Unlock')");
  await expect(page.locator("text=Incorrect PIN")).toBeVisible();

  await page.getByLabel("PIN").fill(PIN);
  await page.click("button:has-text('Unlock')");
  await expect(page.locator("text=Claria is locked")).not.toBeVisible();
  await expect(page.locator("text=Auto-lock hides Claria")).toBeVisible();
});
