import { expect, test, type Page } from "@playwright/test";
import { SOCK_DROP_TIMINGS as T } from "../claria-desktop-frontend/src/lib/sockDrop.js";
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";
const SOCK_BUTTON = "Drop a sock for Lucia";
const pageErrors = new WeakMap<Page, string[]>();
type Phase = "drop" | "walk-in" | "grab" | "shake" | "walk-off";

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  pageErrors.set(page, errors);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript({ content: buildInitScript({ configured: true }) });
  // The clock drives the real component's timeouts. CSS artwork belongs in
  // the separate visual proof; these tests assert the lifecycle it decorates.
  const now = new Date("2026-10-27T12:00:00Z");
  await page.clock.install({ time: now });
  await page.clock.pauseAt(now.getTime() + 1000);
});

test.afterEach(async ({ page }) => {
  expect(pageErrors.get(page)).toEqual([]);
});

async function openClient(page: Page, enabled = true) {
  if (enabled) {
    await page.addInitScript(() => {
      window.localStorage.setItem("claria.distraction_mode", "true");
    });
  }
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe", { exact: true }).click();
}

async function goHomeFromClient(page: Page) {
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await page.getByRole("button", { name: "Back", exact: true }).click();
}

async function openPreference(page: Page) {
  await page.getByTitle("Preferences", { exact: true }).click();
  await page.getByRole("button", { name: "General", exact: true }).click();
  return page.getByLabel("Enable distraction mode");
}

async function reachPhase(page: Page, target: Phase, elapsedDropMs = 0) {
  const overlay = page.getByTestId("sock-drop");
  const shakes = Number(await overlay.getAttribute("data-shakes"));
  expect(Number.isInteger(shakes)).toBe(true);
  expect(shakes).toBeGreaterThanOrEqual(3);
  expect(shakes).toBeLessThanOrEqual(8);
  const steps: Array<[Phase, number]> = [
    ["drop", T.dropMs - elapsedDropMs],
    ["walk-in", T.walkInMs],
    ["grab", T.grabMs],
    ["shake", T.shakeMsPerShake * shakes],
    ["walk-off", T.walkOffMs],
  ];
  for (const [phase, duration] of steps) {
    await expect(overlay).toHaveCount(1);
    await expect(overlay).toHaveAttribute("data-phase", phase);
    await expect(page.getByRole("button", { name: SOCK_BUTTON })).toBeDisabled();
    if (phase === target) return duration;
    // Each timeout arms the next one in a React effect. Advance and observe
    // one phase at a time so no phase can be skipped by a large clock jump.
    await page.clock.runFor(duration);
  }
  throw new Error(`Unknown sock-drop phase: ${target}`);
}

async function finishDrop(page: Page, elapsedDropMs = 0) {
  await reachPhase(page, "walk-off", elapsedDropMs);
  await page.clock.runFor(T.walkOffMs - 1);
  await expect(page.getByTestId("sock-drop")).toHaveAttribute(
    "data-phase", "walk-off",
  );
  await expect(page.getByRole("button", { name: SOCK_BUTTON })).toBeDisabled();
  await page.clock.runFor(1);
  await expect(page.getByTestId("sock-drop")).toHaveCount(0);
  await expect(page.getByRole("button", { name: SOCK_BUTTON })).toBeEnabled();
}

test("the preference starts off and survives repeated enable and disable cycles", async ({
  page,
}) => {
  await openClient(page, false);
  await expect(page.getByRole("button", { name: SOCK_BUTTON })).toHaveCount(0);
  await expect(page.getByTestId("sock-drop")).toHaveCount(0);
  await goHomeFromClient(page);

  const preference = await openPreference(page);
  await expect(preference).not.toBeChecked();
  for (const enabled of [true, false, true, false, true]) {
    await preference.setChecked(enabled);
    await expect(preference).toBeChecked({ checked: enabled });
    expect(
      await page.evaluate(() => localStorage.getItem("claria.distraction_mode")),
    ).toBe(String(enabled));
  }
  await page.reload();
  await expect(await openPreference(page)).toBeChecked();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe", { exact: true }).click();
  await expect(page.getByRole("button", { name: SOCK_BUTTON })).toBeEnabled();
  await page.getByRole("button", { name: SOCK_BUTTON }).click();
  await expect(page.getByTestId("sock-drop")).toHaveCount(1);

  await goHomeFromClient(page);
  await (await openPreference(page)).uncheck();
  await expect(page.getByTestId("sock-drop")).toHaveCount(0);
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe", { exact: true }).click();
  await expect(page.getByRole("button", { name: SOCK_BUTTON })).toHaveCount(0);
  await page.reload();
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe", { exact: true }).click();
  await expect(page.getByRole("button", { name: SOCK_BUTTON })).toHaveCount(0);
  await expect(page.getByTestId("sock-drop")).toHaveCount(0);
});

test("one activation owns the complete sequence and can run again after cleanup", async ({
  page,
}) => {
  await openClient(page);
  const button = page.getByRole("button", { name: SOCK_BUTTON });
  for (let cycle = 0; cycle < 2; cycle++) {
    await button.click();
    await expect(button).toBeDisabled();
    const bounds = await button.boundingBox();
    expect(bounds).not.toBeNull();
    if (!bounds) throw new Error("The enabled sock control has no bounds");
    // Physical clicks on the disabled trigger must not queue a second drop.
    const x = bounds.x + bounds.width / 2;
    const y = bounds.y + bounds.height / 2;
    await page.mouse.click(x, y);
    await page.mouse.click(x, y);
    await expect(page.getByTestId("sock-drop")).toHaveCount(1);
    await finishDrop(page);
    await page.clock.runFor(T.dropMs);
    await expect(page.getByTestId("sock-drop")).toHaveCount(0);
  }
});

test("keyboard activation leaves the workspace controls usable and outside the overlay", async ({
  page,
}) => {
  await openClient(page);
  const button = page.getByRole("button", { name: SOCK_BUTTON });
  await button.focus();
  await page.keyboard.press("Enter");
  const overlay = page.getByTestId("sock-drop");
  await expect(overlay).toHaveAttribute("aria-hidden", "true");
  await expect(overlay).toHaveCSS("pointer-events", "none");
  await expect(
    overlay.locator("button, input, textarea, select, a[href], [tabindex]"),
  ).toHaveCount(0);

  const record = page.getByRole("tab", { name: "Record", exact: true });
  const chat = page.getByRole("tab", { name: "Chat", exact: true });
  await record.focus();
  await page.keyboard.press("ArrowRight");
  await expect(chat).toBeFocused();
  await expect(chat).toHaveAttribute("aria-selected", "true");
  await expect(page.getByText("Start the conversation.")).toBeVisible();
  await page.keyboard.press("Home");
  await expect(record).toBeFocused();
  await expect(record).toHaveAttribute("aria-selected", "true");
  await finishDrop(page);
  await expect(record).toBeFocused();

  await button.focus();
  await page.keyboard.press("Space");
  await expect(overlay).toHaveCount(1);
  await finishDrop(page);
});

test("resizing an active drop keeps its viewport overlay and the page interactive", async ({
  page,
}) => {
  await openClient(page);
  await page.getByRole("button", { name: SOCK_BUTTON }).click();
  await reachPhase(page, "walk-in");
  const overlay = page.getByTestId("sock-drop");
  for (const viewport of [
    { width: 1280, height: 800 },
    { width: 960, height: 720 },
  ]) {
    await page.setViewportSize(viewport);
    await expect(overlay).toHaveCount(1);
    expect(await overlay.boundingBox()).toEqual({ x: 0, y: 0, ...viewport });
    await page.getByRole("tab", { name: "Chat", exact: true }).click();
    await expect(page.getByText("Start the conversation.")).toBeVisible();
    await page.getByRole("tab", { name: "Record", exact: true }).click();
  }
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(overlay).toHaveCount(0);
});

for (const phase of ["drop", "walk-in", "grab", "shake", "walk-off"] as const) {
  test(`leaving during ${phase} removes the overlay and does not poison the next activation`, async ({
    page,
  }) => {
    await openClient(page);
    await page.getByRole("button", { name: SOCK_BUTTON }).click();
    const phaseDuration = await reachPhase(page, phase);
    // Leave just before the old callback is due, then start a replacement
    // before that deadline. Stale work must not affect the new activation.
    await page.clock.runFor(phaseDuration - 1);
    await expect(page.getByTestId("sock-drop")).toHaveAttribute("data-phase", phase);
    await page.getByRole("button", { name: "Back", exact: true }).click();
    await expect(page.getByTestId("sock-drop")).toHaveCount(0);
    await page.getByText("Jane Doe", { exact: true }).click();
    const button = page.getByRole("button", { name: SOCK_BUTTON });
    await expect(button).toBeEnabled();
    await button.click();
    await page.clock.runFor(1);
    await expect(page.getByTestId("sock-drop")).toHaveCount(1);
    await expect(page.getByTestId("sock-drop")).toHaveAttribute("data-phase", "drop");
    await expect(button).toBeDisabled();
    await finishDrop(page, 1);
  });
}

test("Lucia preserves unsent writing, local document edits, and typing focus", async ({
  page,
}) => {
  await openClient(page);
  await page.getByRole("tab", { name: "Writing", exact: true }).click();
  const canvas = page.getByTestId("accepted-report-canvas");
  await expect(canvas).toContainText("Untitled report");
  // Importing the local mock template supplies real document content without
  // starting a chat, planning, drafting, or review operation.
  await page.getByLabel("Writer template").selectOption({
    label: "Sectioned Evaluation Template",
  });
  await page.getByRole("button", { name: "Apply template", exact: true }).click();
  await expect(canvas).toContainText("Template referral text.");
  await page.getByRole("tab", { name: "Write with Claude", exact: true }).click();
  const instruction = page.getByLabel("Writing instruction");
  await instruction.fill("Keep this unsent guidance.");
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  const title = page.getByLabel("Report title", { exact: true });
  await title.fill("Local report title");
  const commands = () =>
    page.evaluate(() => {
      const mock = window as unknown as {
        __REPORT_INVOCATIONS__: Array<{
          cmd: string;
          args: Record<string, unknown>;
        }>;
        __CHAT_COMMANDS__: string[];
      };
      return { reports: mock.__REPORT_INVOCATIONS__, chats: mock.__CHAT_COMMANDS__ };
    });
  const before = await commands();

  await page.getByRole("button", { name: SOCK_BUTTON }).click();
  await title.click();
  await page.keyboard.press("End");
  await page.keyboard.type(" continued");
  await expect(title).toHaveText("Local report title continued");
  await instruction.focus();
  await page.keyboard.press("End");
  await page.keyboard.type(" Still typing.");
  await finishDrop(page);

  await expect(instruction).toBeFocused();
  await expect(instruction).toHaveValue(
    "Keep this unsent guidance. Still typing.",
  );
  await expect(title).toHaveText("Local report title continued");
  const editor = page.getByTestId("inline-report-editor");
  await expect(editor).toContainText("Template referral text.");
  await expect(editor).toContainText("Template background text.");
  await expect(editor).toContainText("Template summary text.");
  await expect(
    page.getByRole("tab", { name: "Writing", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  expect(await commands()).toEqual(before);
});
