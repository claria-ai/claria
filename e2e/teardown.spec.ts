import { expect, test } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";

const ADMIN_ACCESS_KEY = "AKIAIOSFODNN7EXAMPLE";
const ADMIN_SECRET_KEY = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

test.beforeEach(async ({ page }) => {
  await page.addInitScript({ content: buildInitScript({ configured: true }) });
});

test("full teardown requires and discards elevated credentials", async ({
  page,
}) => {
  await page.goto(BASE_URL);
  await page.getByTitle("AWS configuration").click();
  await expect(
    page.getByRole("heading", { name: "AWS Infrastructure" }),
  ).toBeVisible();

  await page.getByText("Advanced", { exact: true }).click();
  await page
    .getByRole("button", { name: "Destroy All Resources", exact: true })
    .click();

  await expect(
    page.getByText("Permanent teardown requires elevated credentials"),
  ).toBeVisible();
  await expect(
    page.getByText("Claria's normal credentials cannot erase version history", {
      exact: false,
    }),
  ).toBeVisible();

  const accessKey = page.getByPlaceholder("Admin Access Key ID");
  const secretKey = page.getByPlaceholder("Admin Secret Access Key");
  const destroy = page.getByRole("button", {
    name: "Permanently Destroy All Resources",
  });

  await expect(destroy).toBeDisabled();
  await accessKey.fill(ADMIN_ACCESS_KEY);
  await secretKey.fill(ADMIN_SECRET_KEY);
  await expect(destroy).toBeEnabled();

  // Cancelling clears secrets rather than retaining elevated credentials in
  // the component for a later action.
  await page.getByRole("button", { name: "Cancel" }).click();
  await page.getByText("Advanced", { exact: true }).click();
  await page
    .getByRole("button", { name: "Destroy All Resources", exact: true })
    .click();
  await expect(accessKey).toHaveValue("");
  await expect(secretKey).toHaveValue("");

  await accessKey.fill(ADMIN_ACCESS_KEY);
  await secretKey.fill(ADMIN_SECRET_KEY);
  await destroy.click();

  await expect(
    page.getByRole("button", { name: "Create New System" }),
  ).toBeVisible();

  const invocations = await page.evaluate(
    () =>
      (
        window as unknown as {
          __PROVISION_INVOCATIONS__: Array<{
            cmd: string;
            args: Record<string, unknown>;
          }>;
        }
      ).__PROVISION_INVOCATIONS__,
  );
  expect(invocations).toEqual([
    {
      cmd: "destroy",
      args: {
        elevatedCredentials: {
          type: "inline",
          access_key_id: ADMIN_ACCESS_KEY,
          secret_access_key: ADMIN_SECRET_KEY,
          session_token: null,
        },
      },
    },
  ]);
});
