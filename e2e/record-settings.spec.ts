import { expect, test } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";

test.beforeEach(async ({ page }) => {
  await page.addInitScript({ content: buildInitScript({ configured: true }) });
});

test("record settings show statistics and rename the client", async ({
  page,
}) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();

  const settings = page.getByRole("button", { name: "Record settings" });
  await settings.click();
  const recordTab = page.locator('[data-tab="record"]');
  await expect(settings).toHaveAttribute("aria-pressed", "true");
  await expect(settings).toHaveClass(/bg-blue-100/);
  await expect(recordTab).toHaveAttribute("aria-selected", "false");
  await expect(recordTab).toHaveClass(/bg-white/);
  await expect(
    page.getByRole("heading", { name: "Record settings" })
  ).toBeVisible();
  await expect(page.getByText("3 files")).toBeVisible();
  await expect(page.getByText("5.5 MB")).toBeVisible();
  await expect(page.getByText("Including history: 7.0 MB")).toBeVisible();
  await expect(page.getByText("Aug 1, 2026", { exact: true })).toBeVisible();
  await expect(page.getByText("Record details")).toHaveCount(0);
  await expect(page.getByText("Record statistics")).toHaveCount(0);
  await expect(
    page.getByRole("table", { name: "Record name history" })
  ).toBeVisible();
  await expect(page.getByRole("cell", { name: "Jane A. Doe" })).toBeVisible();

  const name = page.getByRole("textbox", { name: "Record name", exact: true });
  await name.fill("Jane Smith");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Record name updated.")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Jane Smith" })).toBeVisible();
  await expect(page.getByRole("cell", { name: "Jane Smith" })).toBeVisible();

  await page.getByRole("button", { name: "Back" }).click();
  await expect(page.getByText("Jane Smith")).toBeVisible();
});
