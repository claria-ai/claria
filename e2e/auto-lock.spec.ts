/**
 * Set up auto-lock, lock on demand, and unlock with the PIN.
 *
 * The occlusion check is the point of the test: the React tree under the
 * overlay stays mounted so an unsaved draft survives a lock, which makes
 * "nothing behind it is reachable at any sampled point" the only honest way
 * to assert the screen is actually covered.
 */

import { expect, test } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";

const PIN = "482913";

test.beforeEach(async ({ page }) => {
  await page.addInitScript({ content: buildInitScript({ configured: true }) });
});

test("auto-lock setup, manual lock, and PIN unlock", async ({ page }) => {
  await page.goto(BASE_URL);
  await page.click('[data-page="preferences"]');
  await page.getByRole("button", { name: "Security" }).click();

  // ── Turn it on ───────────────────────────────────────────────────────
  await page.getByRole("button", { name: "Set up auto-lock" }).click();
  await page.getByLabel("Choose a PIN (6–12 digits)").fill(PIN);
  await page.getByLabel("Repeat PIN").fill(PIN);
  await page.getByRole("button", { name: "Turn on auto-lock" }).click();

  await expect(page.getByRole("button", { name: "Lock now" })).toBeVisible();

  // ── Lock on demand ───────────────────────────────────────────────────
  await page.getByRole("button", { name: "Lock now" }).click();
  await expect(page.getByTestId("lock-screen")).toBeVisible();

  const covered = await page.evaluate(() => {
    const points: [number, number][] = [
      [window.innerWidth / 2, window.innerHeight / 2],
      [10, 10],
      [window.innerWidth - 10, window.innerHeight - 10],
    ];
    return points.every((point) => {
      const element = document.elementFromPoint(point[0], point[1]);
      return element?.closest('[data-testid="lock-screen"]') != null;
    });
  });
  expect(covered).toBe(true);

  // ── The wrong PIN says so and stays locked ───────────────────────────
  await page.getByLabel("PIN").fill("999999");
  await page.getByRole("button", { name: "Unlock" }).click();
  await expect(page.getByRole("alert")).toContainText("Incorrect PIN");
  await expect(page.getByTestId("lock-screen")).toBeVisible();

  // ── The right one gets back in ───────────────────────────────────────
  await page.getByLabel("PIN").fill(PIN);
  await page.getByRole("button", { name: "Unlock" }).click();
  await expect(page.getByTestId("lock-screen")).toBeHidden();
  await expect(page.getByRole("button", { name: "Lock now" })).toBeVisible();
});
