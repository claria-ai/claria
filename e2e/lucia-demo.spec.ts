import { expect, test } from "@playwright/test";
import manifest from "../claria-desktop-frontend/src/assets/lucia/manifest.json" with { type: "json" };

const duration = (name: keyof typeof manifest.clips) => manifest.clips[name].frame_durations_ms.reduce((a, b) => a + b, 0);

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


test("local previews expose real Companion actions without changing modes or natural scheduling", async ({ page }) => {
  await page.goto(`${BASE_URL}/lucia-demo.html`);
  const scene = page.getByTestId("lucia-scene");
  const preview = page.getByLabel("Preview Lucia action", { exact: true });
  await expect(scene).toHaveAttribute("data-behavior", "idle");
  for (const behavior of ["tailwag", "ear-perk", "twirl", "skateboard", "stretch"]) {
    await preview.selectOption(behavior);
    await expect(scene).toHaveAttribute("data-behavior", behavior);
    await expect(scene).toHaveAttribute("data-behavior", behavior === "stretch" ? "bored" : "idle", { timeout: 6000 });
  }
  await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-image", /bored-strip/);
  await page.getByLabel("Lucia mode", { exact: true }).selectOption("gaze");
  await expect(preview).toBeDisabled();
  await page.getByLabel("Lucia mode", { exact: true }).selectOption("companion");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(preview).toBeDisabled();
});

test("natural Companion quiet and play deadlines reach their mapped strips with a controlled successful roll", async ({ page }) => {
  const now = new Date("2026-11-02T12:00:00Z");
  await page.clock.install({ time: now });
  await page.clock.pauseAt(now.getTime() + 1000);
  await page.addInitScript(() => { Math.random = () => 0; });
  await page.goto(`${BASE_URL}/lucia-demo.html`);
  const scene = page.getByTestId("lucia-scene");
  await expect(scene).toHaveAttribute("data-behavior", "tailwag");
  await page.clock.runFor(duration("tailwag"));
  await expect(scene).toHaveAttribute("data-behavior", "idle");
  await page.clock.fastForward(30_000);
  await expect(scene).toHaveAttribute("data-behavior", "stretch");
  await page.clock.runFor(duration("stretch"));
  await expect(scene).toHaveAttribute("data-behavior", "bored");
  await page.clock.fastForward(120_000);
  await expect(scene).toHaveAttribute("data-behavior", "twirl");
  await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-image", /twirl/);
  await page.clock.runFor(duration("twirl") + duration("stretch"));
  await expect(scene).toHaveAttribute("data-behavior", "bored");
  await page.clock.fastForward(600_000);
  await expect(scene).toHaveAttribute("data-behavior", "skateboard");
  await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-image", /skateboard-left/);
});

test("decoded canvas stays nonblank across every sampled sock-switch frame and keeps idle registration fixed", async ({ page }) => {
  await page.goto(`${BASE_URL}/lucia-demo.html`);
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "idle");
  const initial = await page.getByTestId("lucia-sprite").boundingBox();
  const idlePixels = await page.locator('[data-testid="lucia-sprite"] canvas').evaluate((canvas) => (canvas as HTMLCanvasElement).toDataURL());
  await page.waitForTimeout(1200);
  expect(await page.getByTestId("lucia-sprite").boundingBox()).toEqual(initial);
  expect(await page.locator('[data-testid="lucia-sprite"] canvas').evaluate((canvas) => (canvas as HTMLCanvasElement).toDataURL())).toBe(idlePixels);
  await page.evaluate(() => {
    type Sample = { behavior: string | undefined; frame: string | undefined; opaque: number; nativeWidth: number; nativeHeight: number; width: number; height: number; sock: boolean };
    const state = window as typeof window & { luciaSwitchSamples: Sample[]; stopLuciaSampling: boolean };
    state.luciaSwitchSamples = [];
    state.stopLuciaSampling = false;
    const sample = () => {
      const sprite = document.querySelector<HTMLElement>('[data-testid="lucia-sprite"]')!;
      const canvas = sprite.querySelector("canvas")!;
      const pixels = canvas.getContext("2d")!.getImageData(0, 0, 192, 208).data;
      let opaque = 0;
      for (let i = 3; i < pixels.length; i += 4) if (pixels[i] >= 128) opaque++;
      const rect = sprite.getBoundingClientRect();
      state.luciaSwitchSamples.push({ behavior: document.querySelector<HTMLElement>('[data-testid="lucia-scene"]')?.dataset.behavior, frame: sprite.dataset.frame, opaque, nativeWidth: canvas.width, nativeHeight: canvas.height, width: rect.width, height: rect.height, sock: !!document.querySelector('[data-testid="lucia-sock"]') });
      if (!state.stopLuciaSampling) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  });
  await page.getByRole("button", { name: "Drop a sock for Lucia" }).click();
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "carry-away", { timeout: 6000 });
  await page.waitForTimeout(200);
  const samples = await page.evaluate(() => {
    const state = window as typeof window & { stopLuciaSampling: boolean; luciaSwitchSamples: Array<{ behavior: string; frame: string; opaque: number; nativeWidth: number; nativeHeight: number; width: number; height: number; sock: boolean }> };
    state.stopLuciaSampling = true;
    return state.luciaSwitchSamples;
  });
  expect(samples.length).toBeGreaterThan(60);
  for (const sample of samples) {
    expect(sample.opaque).toBeGreaterThan(10_000);
    expect([sample.nativeWidth, sample.nativeHeight]).toEqual([192, 208]);
    expect(sample.width).toBeCloseTo(124.8, 1);
    expect(sample.height).toBeCloseTo(135.2, 1);
    if (["pickup", "carry-away"].includes(sample.behavior)) expect(sample.sock).toBe(false);
  }
  for (const behavior of ["pickup", "carry-away"]) {
    expect(samples.find((sample) => sample.behavior === behavior)?.frame).toBe("0:0");
  }
});
