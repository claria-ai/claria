import { expect, test } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";

test("webview console errors reach the desktop logging command", async ({
  page,
}) => {
  await page.addInitScript({ content: buildInitScript({ configured: true }) });
  await page.goto(BASE_URL);

  await page.evaluate(() => {
    console.error("Synthetic webview support diagnostic");
  });

  await expect
    .poll(() =>
      page.evaluate(() =>
        (
          window as unknown as {
            __FRONTEND_LOGS__: Array<{ level: string; message: string }>;
          }
        ).__FRONTEND_LOGS__.slice()
      )
    )
    .toContainEqual({
      level: "error",
      message:
        "Webview console.error: Synthetic webview support diagnostic",
    });
});
