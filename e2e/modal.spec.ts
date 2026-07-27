/**
 * E2E test: the native `<dialog>` modal shell, in a real browser.
 *
 * `Modal.test.tsx` covers the component's own logic under happy-dom. What it
 * cannot cover is the user agent's half of the contract, because happy-dom
 * models `<dialog>` as attribute bookkeeping: it never turns Escape into a
 * `cancel` event, never puts the dialog in the top layer, never contains
 * focus, and never paints `::backdrop`. Stubbing any of that would prove
 * nothing, so it is asserted here instead.
 *
 * The delete confirmation on the client list is the shortest path to a real
 * modal. It is `closeOnBackdropClick={false}`, which also makes it the right
 * place to check that a click on the scrim really does reach the element the
 * component guards on.
 */

import { expect, test } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = "http://localhost:1420";

const SEEDED_CONFIG = {
  region: "us-east-1",
  system_name: "Modal Spec",
  account_id: "185735714230",
  created_at: "2026-01-01T00:00:00Z",
  credential_type: "inline",
  profile_name: null,
  access_key_hint: "AKIA...0001",
  preferred_model_id: null,
  cost_explorer_enabled: false,
  hourly_cost_data: false,
};

/**
 * Start from a configured system with one client, so the run lands on the
 * client list rather than walking onboarding.
 */
const SEED_SCRIPT = `
  (() => {
    const inner = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
      if (cmd === "has_config") return true;
      if (cmd === "load_config") return ${JSON.stringify(SEEDED_CONFIG)};
      if (cmd === "list_clients") {
        return [{ id: "modal-spec-client", name: "Ada Lovelace", created_at: "2026-01-01T00:00:00Z" }];
      }
      return inner(cmd, args);
    };
  })();
`;

test.beforeEach(async ({ page }) => {
  await page.addInitScript({ content: buildInitScript() });
  await page.addInitScript({ content: SEED_SCRIPT });
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await expect(page.getByText("Ada Lovelace")).toBeVisible();
});

/** Open the delete confirmation for the seeded client. */
async function openConfirmation(page: import("@playwright/test").Page) {
  await page.getByTitle("Delete client").click();
  await expect(page.getByRole("heading", { name: "Delete client?" })).toBeVisible();
}

test("the dialog really is a modal in the top layer", async ({ page }) => {
  await openConfirmation(page);

  // `:modal` only matches a dialog opened with showModal(). If the component
  // ever regressed to show() or to a plain div, this is what would notice.
  const isModal = await page.evaluate(
    () => document.querySelector("dialog")?.matches(":modal") ?? false
  );
  expect(isModal).toBe(true);

  // ::backdrop paints the scrim. Nothing else in the app supplies one.
  const backdropColor = await page.evaluate(() => {
    const dialog = document.querySelector("dialog");
    if (!dialog) return null;
    return getComputedStyle(dialog, "::backdrop").backgroundColor;
  });
  expect(backdropColor).not.toBe("rgba(0, 0, 0, 0)");
});

test("Escape closes the modal and it reopens", async ({ page }) => {
  // The premise the unit suite's shim encodes: a real Escape produces a
  // `cancel` event that the component turns into an onClose.
  await openConfirmation(page);

  await page.keyboard.press("Escape");
  await expect(page.getByRole("heading", { name: "Delete client?" })).toBeHidden();

  // The desync footgun: if the browser had closed the dialog on its own,
  // React would still think it was open and this second open would do nothing.
  await openConfirmation(page);
});

test("Escape survives several cycles", async ({ page }) => {
  for (let i = 0; i < 3; i++) {
    await openConfirmation(page);
    await page.keyboard.press("Escape");
    await expect(page.getByRole("heading", { name: "Delete client?" })).toBeHidden();
  }
});

test("a click on the scrim does not close a delete confirmation", async ({ page }) => {
  await openConfirmation(page);

  // Top-left of the viewport, well clear of the card. In a browser this lands
  // on the full-viewport container inside the dialog, which is the element
  // the component's target guard is written against.
  await page.mouse.click(8, 8);

  await expect(page.getByRole("heading", { name: "Delete client?" })).toBeVisible();
});

test("focus never reaches the page behind the dialog", async ({ page }) => {
  await openConfirmation(page);

  // Tab several times round the cycle. Focus containment is the whole reason
  // the hand-rolled overlays were replaced, so nothing on the page behind may
  // take focus. The cycle wraps through the document itself — activeElement
  // is `body` for one step — which is the user agent's own behaviour and not
  // a control on the page.
  const visited: string[] = [];
  for (let i = 0; i < 12; i++) {
    await page.keyboard.press("Tab");
    visited.push(
      await page.evaluate(() => {
        const dialog = document.querySelector("dialog");
        const active = document.activeElement;
        if (active == null) return "none";
        if (active === document.body) return "document";
        return dialog?.contains(active) ? "inside" : `OUTSIDE: ${active.outerHTML}`;
      })
    );
  }

  expect(visited.filter((v) => v.startsWith("OUTSIDE"))).toEqual([]);
  // And the dialog's own controls really were in the cycle, so this is not
  // passing because Tab did nothing at all.
  expect(visited).toContain("inside");
});

test("the page behind is held still while the modal is open", async ({ page }) => {
  const lockedBefore = await page.evaluate(() =>
    document.body.classList.contains("overflow-hidden")
  );
  expect(lockedBefore).toBe(false);

  await openConfirmation(page);
  expect(
    await page.evaluate(() => document.body.classList.contains("overflow-hidden"))
  ).toBe(true);

  await page.keyboard.press("Escape");
  await expect(page.getByRole("heading", { name: "Delete client?" })).toBeHidden();
  expect(
    await page.evaluate(() => document.body.classList.contains("overflow-hidden"))
  ).toBe(false);
});
