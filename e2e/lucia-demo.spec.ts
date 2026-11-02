import { expect, test } from "@playwright/test";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";

test("local demo opens the real workspace and visible Lucia controls without AWS setup or remote requests", async ({ page }) => {
  const remote: string[] = [];
  const errors: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).origin !== new URL(BASE_URL).origin) remote.push(request.url());
  });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${BASE_URL}/lucia-demo.html`);
  await expect(page.getByRole("region", { name: "Local demo" })).toContainText("simulated activity");
  await expect(page.getByRole("heading", { name: "Jane Doe — synthetic demo" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Record", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByLabel("Lucia mode")).toBeVisible();
  await expect(page.getByTestId("lucia-scene")).toBeVisible();
  await expect(page.getByTitle("AWS configuration", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Create New System" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Record Memo", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Simulate inference", exact: true }).click();
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "reading");
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "idle", { timeout: 10_000 });
  await page.getByRole("tab", { name: "Chat", exact: true }).click();
  await expect(page.getByRole("textbox")).toBeVisible();
  await page.getByRole("tab", { name: "Writing", exact: true }).click();
  await expect(page.getByTestId("accepted-report-canvas")).toBeVisible();
  expect(remote).toEqual([]);
  expect(errors).toEqual([]);
});

test("demo cannot replace an existing native bridge", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript(() => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: { demoGuardSentinel: true }, configurable: true });
  });
  await page.goto(`${BASE_URL}/lucia-demo.html`);
  await expect.poll(() => errors.some((error) => error.includes("local development browser"))).toBe(true);
  await expect(page.getByTestId("lucia-scene")).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { demoGuardSentinel: boolean } }).__TAURI_INTERNALS__.demoGuardSentinel)).toBe(true);
});
