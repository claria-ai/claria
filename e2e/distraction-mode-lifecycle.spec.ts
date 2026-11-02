import { expect, test, type Page } from "@playwright/test";
import { writeFile } from "node:fs/promises";
import manifest from "../claria-desktop-frontend/src/assets/lucia/manifest.json" with { type: "json" };
import { buildInitScript } from "./tauri-mock.js";

const BASE_URL = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";
const SOCK_BUTTON = "Drop a sock for Lucia";
const pageErrors = new WeakMap<Page, string[]>();
type Phase = "chase" | "pickup" | "carry-away";
const duration = (name: keyof typeof manifest.clips) => manifest.clips[name].frame_durations_ms.reduce((a, b) => a + b, 0);
const timings = { chase: duration("running-right") * 2, pickup: duration("pickup"), "carry-away": duration("carry-away") * 3 };

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  pageErrors.set(page, errors);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript({ content: buildInitScript({ configured: true }) });
  const now = new Date("2026-10-27T12:00:00Z");
  await page.clock.install({ time: now });
  await page.clock.pauseAt(now.getTime() + 1000);
});
test.afterEach(async ({ page }) => { expect(pageErrors.get(page)).toEqual([]); });

async function openClient(page: Page, enabled = true) {
  if (enabled) await page.addInitScript(() => localStorage.setItem("claria.distraction_mode", "true"));
  await page.goto(BASE_URL);
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe", { exact: true }).click();
  if (enabled) await expect(page.getByTestId("lucia-scene")).toBeVisible();
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
async function reachPhase(page: Page, target: Phase) {
  for (const phase of ["chase", "pickup", "carry-away"] as const) {
    await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", phase);
    await expect(page.getByRole("button", { name: SOCK_BUTTON })).toBeDisabled();
    await expect(page.getByTestId("lucia-sock")).toHaveCount(phase === "chase" ? 1 : 0);
    if (target === phase) return timings[phase];
    await page.clock.runFor(timings[phase]);
  }
  throw new Error(`Unknown phase: ${target}`);
}
async function finishDrop(page: Page) {
  await reachPhase(page, "carry-away");
  await page.clock.runFor(timings["carry-away"]);
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "idle");
  await expect(page.getByRole("button", { name: SOCK_BUTTON })).toBeEnabled();
}
// Lucia's controls are always visible in her rail; nothing to open.
async function controls(page: Page) { await page.getByRole("group", { name: "Lucia controls" }).waitFor(); }
async function screenshot(page: Page, name: string) {
  if (process.env.CLARIA_LUCIA_ARTIFACT_DIR) {
    await page.screenshot({ path: `${process.env.CLARIA_LUCIA_ARTIFACT_DIR}/${name}.png` });
  }
}

test("default off, repeated toggles and immediate disable leave no pet interception", async ({ page }) => {
  await openClient(page, false);
  await expect(page.getByTestId("lucia-rail")).toHaveCount(0);
  await goHomeFromClient(page);
  const preference = await openPreference(page);
  await expect(preference).not.toBeChecked();
  for (const enabled of [true, false, true, false, true]) await preference.setChecked(enabled);
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe", { exact: true }).click();
  await page.getByRole("button", { name: SOCK_BUTTON }).click();
  await controls(page);
  await page.getByRole("button", { name: "Hide Lucia" }).click();
  await expect(page.getByTestId("lucia-rail")).toHaveCount(0);
  await page.clock.runFor(600_000);
  await expect(page.getByTestId("lucia-scene")).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("claria.distraction_mode"))).toBe("false");
});

test("keyboard sock action is single-owned, switches art at pickup and leaves navigation usable", async ({ page }) => {
  await openClient(page);
  await page.clock.runFor(duration("tailwag"));
  await screenshot(page, "idle");
  const button = page.getByRole("button", { name: SOCK_BUTTON });
  await button.focus();
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("lucia-scene")).toHaveCount(1);
  await reachPhase(page, "pickup");
  await screenshot(page, "sock-pickup");
  await page.clock.runFor(timings.pickup);
  await screenshot(page, "sock-carry");
  const record = page.getByRole("tab", { name: "Record", exact: true });
  await record.focus();
  await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("tab", { name: "Chat", exact: true })).toBeFocused();
  const panel = await page.getByRole("tabpanel", { name: "Chat", exact: true }).boundingBox();
  const rail = await page.getByTestId("lucia-rail").boundingBox();
  expect(panel && rail && panel.y + panel.height <= rail.y).toBe(true);
  await page.clock.runFor(timings["carry-away"]);
  await expect(button).toBeEnabled();
});

for (const phase of ["chase", "pickup", "carry-away"] as const) {
  test(`leaving during ${phase} clears timers without poisoning the replacement scene`, async ({ page }) => {
    await openClient(page);
    await page.getByRole("button", { name: SOCK_BUTTON }).click();
    await reachPhase(page, phase);
    await page.getByRole("button", { name: "Back", exact: true }).click();
    await expect(page.getByTestId("lucia-scene")).toHaveCount(0);
    await page.getByText("Jane Doe", { exact: true }).click();
    await page.getByRole("button", { name: SOCK_BUTTON }).click();
    await finishDrop(page);
  });
}

test("all sixteen gaze directions keep the seated body at one ground anchor", async ({ page }) => {
  await openClient(page);
  await controls(page);
  await page.getByRole("group", { name: "Lucia controls" }).getByRole("button", { name: "Watch", exact: true }).click();
  await controls(page);
  const sprite = page.getByTestId("lucia-sprite");
  const bounds = await sprite.boundingBox();
  if (!bounds) throw new Error("Lucia has no bounds");
  let directional: typeof bounds | null = null;
  for (let direction = 0; direction < 16; direction++) {
    const radians = direction * Math.PI / 8;
    await page.mouse.move(bounds.x + 80 * 0.65 + Math.sin(radians) * 50, bounds.y + 66 * 0.65 - Math.cos(radians) * 50);
    await page.clock.runFor(20);
    await expect(sprite).toHaveAttribute("data-frame", `${9 + Math.floor(direction / 8)}:${direction % 8}`);
    const box = await sprite.boundingBox();
    if (!box) throw new Error("Lucia has no bounds");
    directional ??= box;
    expect(box).toEqual(directional);
    await screenshot(page, `gaze-${direction}`);
  }
  // Neutral (idle cell 0) and every gaze cell share one measured ground line.
  expect(directional).toEqual(bounds);
});

test("real hidden browser tab drops transient work and resumes without a missed-action backlog", async ({ page, context }, testInfo) => {
  test.fixme(true, "Installed headed Chrome reports visible/focused with no lifecycle events after tab switching, even with CDP focus emulation disabled; actual hidden/resume needs manual platform qualification.");
  await page.addInitScript(() => {
    const observations: Array<{ event: string; visibility: string; focused: boolean }> = [];
    Object.assign(window, { __LUCIA_VISIBILITY_OBSERVATIONS__: observations });
    const observe = (event: Event) => observations.push({ event: event.type, visibility: document.visibilityState, focused: document.hasFocus() });
    document.addEventListener("visibilitychange", observe);
    window.addEventListener("focus", observe);
    window.addEventListener("blur", observe);
  });
  await openClient(page);
  await page.getByRole("button", { name: SOCK_BUTTON }).click();
  // Playwright enables Chromium focus emulation by default, making even
  // background tabs report visible/focused. Restore real browser lifecycle here.
  const cdp = await context.newCDPSession(page);
  await cdp.send("Emulation.setFocusEmulationEnabled", { enabled: false });
  const other = await context.newPage();
  try {
    await other.goto("about:blank");
    await other.bringToFront();
    const environment = await page.evaluate(() => ({
      visibility: document.visibilityState, focused: document.hasFocus(),
      observations: (window as unknown as { __LUCIA_VISIBILITY_OBSERVATIONS__: unknown }).__LUCIA_VISIBILITY_OBSERVATIONS__,
    }));
    const evidence = testInfo.outputPath("visibility-preconditions.json");
    await writeFile(evidence, JSON.stringify(environment, null, 2));
    await testInfo.attach("visibility-preconditions", { path: evidence, contentType: "application/json" });
    expect(environment.visibility, "Actual hidden-tab precondition").toBe("hidden");
    expect(environment.focused).toBe(false);
    await expect(page.getByTestId("lucia-scene")).toHaveCount(0);
    await page.clock.fastForward(600_000);
    await page.bringToFront();
    expect(await page.evaluate(() => ({ visibility: document.visibilityState, focused: document.hasFocus() })))
      .toEqual({ visibility: "visible", focused: true });
    await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "tailwag");
    await expect(page.getByTestId("lucia-sock")).toHaveCount(0);
    await page.clock.runFor(duration("tailwag"));
    await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "idle");
  } finally {
    await cdp.send("Emulation.setFocusEmulationEnabled", { enabled: true });
    await cdp.detach();
    await other.close();
  }
});

test("synthetic visibility lifecycle removes the scene and resumes without a missed-action backlog", async ({ page }) => {
  await openClient(page);
  await page.getByRole("button", { name: SOCK_BUTTON }).click();
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect(page.getByTestId("lucia-scene")).toHaveCount(0);
  await expect(page.getByRole("button", { name: SOCK_BUTTON })).toBeDisabled();
  await page.clock.fastForward(600_000);
  await page.evaluate(() => {
    Reflect.deleteProperty(document, "hidden");
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "tailwag");
  await expect(page.getByTestId("lucia-sock")).toHaveCount(0);
  await page.clock.runFor(duration("tailwag"));
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "idle");
});

test("feeding animates each matching starting body, reaches five exactly and resets explicitly", async ({ page }) => {
  await openClient(page);
  await controls(page);
  await page.getByRole("group", { name: "Lucia controls" }).getByRole("button", { name: "Feed", exact: true }).click();
  await screenshot(page, "feeding-0");
  const feed = page.getByRole("button", { name: "Feed a dekopon" });
  for (let count = 0; count < 5; count++) {
    await feed.click();
    await expect(feed).toBeDisabled();
    await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-fed-count", String(count));
    await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-image", `extras/dekopon/${count === 0 ? "feeding" : `feeding-size-${count}`}/feeding-strip.png`);
    await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-frame", "0:0");
    await expect(page.getByTestId("lucia-fruit")).toHaveCount(0);
    await page.clock.runFor(800);
    await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-frame", "0:5");
    await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-fed-count", String(count));
    await page.clock.runFor(duration("feeding") - 800);
    await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-fed-count", String(count + 1));
  }
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "full");
  await page.clock.runFor(duration("full"));
  await screenshot(page, "feeding-5");
  await expect(feed).toBeDisabled();
  await page.getByRole("button", { name: "Start over" }).click();
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-fed-count", "0");
});

test("tracked chat inference enters, loops and puts the newspaper away on completion", async ({ page }) => {
  await openClient(page);
  await page.evaluate(() => {
    const mock = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (cmd: string, args: unknown) => Promise<unknown> };
      __FINISH_LUCIA_CHAT__: () => void;
    };
    const invoke = mock.__TAURI_INTERNALS__.invoke;
    mock.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
      if (cmd === "chat_message") await new Promise<void>((resolve) => { mock.__FINISH_LUCIA_CHAT__ = resolve; });
      return invoke(cmd, args);
    };
  });
  await page.getByRole("tab", { name: "Chat", exact: true }).click();
  await page.getByRole("textbox").fill("Synthetic fixture question");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await page.clock.runFor(200);
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "reading-enter");
  await page.clock.runFor(duration("reading-enter"));
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "reading");
  await screenshot(page, "inference-reading");
  await page.evaluate(() => (window as unknown as { __FINISH_LUCIA_CHAT__: () => void }).__FINISH_LUCIA_CHAT__());
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "reading-exit");
  await page.clock.runFor(duration("reading-exit"));
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "idle");
});

test("reduced motion freezes frames and suppresses travel; failed images stop the scene", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openClient(page);
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-reduced-motion", "true");
  const bounds = await page.getByTestId("lucia-sprite").boundingBox();
  await page.clock.runFor(600_000);
  await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-frame", "0:0");
  expect(await page.getByTestId("lucia-sprite").boundingBox()).toEqual(bounds);
  await screenshot(page, "reduced-motion");
  await controls(page);
  await page.getByRole("group", { name: "Lucia controls" }).getByRole("button", { name: "Feed", exact: true }).click();
  for (let count = 0; count < 5; count++) {
    await page.getByRole("button", { name: "Feed a dekopon" }).click();
    await page.clock.runFor(800);
    await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-image", manifest.stages.image);
    await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-frame", `0:${count}`);
    await page.clock.runFor(duration("feeding") - 800);
    await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-fed-count", String(count + 1));
  }
  await page.clock.runFor(duration("full"));
  await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-frame", "0:5");
  // Vite also serves PNG URL imports as JavaScript modules; fail the image,
  // not the application's module graph on reload.
  await page.route("**/assets/lucia/**/*.png*", (route) =>
    route.request().resourceType() === "image" ? route.abort() : route.continue(),
  );
  await page.reload();
  await page.getByRole("button", { name: "Client Files" }).click();
  await page.getByText("Jane Doe", { exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("artwork could not be loaded");
  await expect(page.getByTestId("lucia-scene")).toHaveCount(0);
});

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
  await title.focus();
  // On macOS End scrolls instead of moving the editing caret. Select/collapse
  // uses real keyboard behavior and deterministically positions it on all hosts.
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.press("ArrowRight");
  await expect(title).toBeFocused();
  await page.keyboard.type(" continued");
  await expect(title).toHaveText("Local report title continued");
  await instruction.focus();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.press("ArrowRight");
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
