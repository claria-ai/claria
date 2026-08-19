import { expect, test } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";

test.beforeEach(async ({ page }) => {
  await page.addInitScript({ content: buildInitScript({ configured: true }) });
  await page.addInitScript(() => {
    window.localStorage.setItem("claria.distraction_mode", "true");
  });
});

test("the quiet header sock summons Lucia into a neck-driven play bow", async ({
  page,
}) => {
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe").click();

  const button = page.getByRole("button", { name: "Drop a sock for Lucia" });
  await expect(button).toBeVisible();
  await expect(button).toHaveClass(/p-1\.5/);
  await expect(button).not.toHaveClass(/border/);
  expect(
    await button.evaluate((element) =>
      element.nextElementSibling?.matches("h2")
    )
  ).toBe(true);

  await button.click();
  const overlay = page.getByTestId("sock-drop");
  await expect(overlay).toHaveAttribute("data-phase", "grab", {
    timeout: 5000,
  });
  await expect(page.getByTestId("sock-dog-body")).toHaveAttribute(
    "data-pose",
    "play-bow"
  );

  await expect(overlay).toHaveAttribute("data-phase", "shake", {
    timeout: 1500,
  });
  await expect(page.getByTestId("sock-dog-motion")).toHaveCSS(
    "animation-name",
    "none"
  );
  await expect(page.getByTestId("sock-dog-neck")).toHaveCSS(
    "animation-name",
    "sock-dog-neck-shake"
  );
});
