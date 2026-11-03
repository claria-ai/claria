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
  await expect(page.getByRole("group", { name: "Lucia controls" })).toBeVisible();
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
    await expect(scene).toHaveAttribute("data-behavior", behavior, { timeout: 6000 });
    await expect(scene).toHaveAttribute("data-behavior", behavior === "stretch" ? "bored" : "idle", { timeout: 6000 });
  }
  await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-image", /bored-strip/);
  await page.getByRole("group", { name: "Lucia controls" }).getByRole("button", { name: "Watch", exact: true }).click();
  await expect(preview).toBeDisabled();
  await page.getByRole("group", { name: "Lucia controls" }).getByRole("button", { name: "Companion", exact: true }).click();
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
  // The skate deadline is three minutes after mount.
  await page.clock.fastForward(60_000);
  await expect(scene).toHaveAttribute("data-behavior", "skateboard");
  await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-image", /skateboard-left/);
});

test("decoded canvas stays nonblank across every sampled sock-switch frame and idle loops on a fixed registration", async ({ page }) => {
  await page.goto(`${BASE_URL}/lucia-demo.html`);
  await expect(page.getByTestId("lucia-scene")).toHaveAttribute("data-behavior", "idle");
  const initial = await page.getByTestId("lucia-sprite").boundingBox();
  const idlePixels = await page.locator('[data-testid="lucia-sprite"] canvas').evaluate((canvas) => (canvas as HTMLCanvasElement).toDataURL());
  await page.waitForTimeout(1200);
  expect(await page.getByTestId("lucia-sprite").boundingBox()).toEqual(initial);
  // Idle animates in place now that its cells keep one silhouette; only the pixels change.
  expect(await page.locator('[data-testid="lucia-sprite"] canvas').evaluate((canvas) => (canvas as HTMLCanvasElement).toDataURL())).not.toBe(idlePixels);
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


test("every-size feeding stays decoded, grounded and count-correct through all five completions", async ({ page }) => {
  await page.goto(`${BASE_URL}/lucia-demo.html`);
  await page.getByRole("group", { name: "Lucia controls" }).getByRole("button", { name: "Feed", exact: true }).click();
  const scene = page.getByTestId("lucia-scene");
  const feed = page.getByRole("button", { name: "Feed a dekopon", exact: true });
  await page.evaluate(() => {
    const samples: Array<{ behavior: string; count: number; image: string; frame: string; opaque: number; width: number; gap: number; duplicateFruit: boolean }> = [];
    const sample = () => {
      const sprite = document.querySelector<HTMLElement>('[data-testid="lucia-sprite"]')!;
      const scene = document.querySelector<HTMLElement>('[data-testid="lucia-scene"]')!;
      const pixels = sprite.querySelector("canvas")!.getContext("2d")!.getImageData(0, 0, 192, 208).data;
      let minX = 192, maxX = -1, bottom = -1, opaque = 0;
      for (let y = 0; y < 208; y++) for (let x = 0; x < 192; x++) {
        if (pixels[(y * 192 + x) * 4 + 3] >= 128) {
          minX = Math.min(minX, x); maxX = Math.max(maxX, x); bottom = y; opaque++;
        }
      }
      const box = sprite.getBoundingClientRect();
      const rail = document.querySelector('[data-testid="lucia-rail"]')!.getBoundingClientRect();
      samples.push({ behavior: scene.dataset.behavior!, count: Number(scene.dataset.fedCount), image: sprite.dataset.image!, frame: sprite.dataset.frame!, opaque, width: maxX - minX + 1, gap: rail.bottom - (box.y + (bottom + 1) * box.height / 208), duplicateFruit: !!document.querySelector('[data-testid="lucia-fruit"]') });
      if (scene.dataset.behavior !== "feeding-idle" || scene.dataset.fedCount !== "5") requestAnimationFrame(sample);
    };
    Object.assign(window, { luciaFeedingSamples: samples });
    requestAnimationFrame(sample);
  });
  for (let count = 0; count < 5; count++) {
    await feed.click();
    await expect(feed).toBeDisabled();
    await feed.evaluate((button) => (button as HTMLButtonElement).click());
    await expect(scene).toHaveAttribute("data-fed-count", String(count));
    await expect(scene).toHaveAttribute("data-fed-count", String(count + 1));
    if (count < 4) await page.waitForTimeout(50); // Let the sampler observe the completed static stage before the next click.
  }
  await expect(scene).toHaveAttribute("data-behavior", "feeding-idle");
  await expect(feed).toBeDisabled();
  await page.waitForTimeout(50);
  const samples = await page.evaluate(() => (window as typeof window & { luciaFeedingSamples: Array<{ behavior: string; count: number; image: string; frame: string; opaque: number; width: number; gap: number; duplicateFruit: boolean }> }).luciaFeedingSamples);
  expect(samples.length).toBeGreaterThan(100);
  expect([...new Set(samples.map((sample) => sample.count))]).toEqual([0, 1, 2, 3, 4, 5]);
  for (const sample of samples) {
    expect(sample.opaque).toBeGreaterThan(10_000);
    expect(sample.duplicateFruit).toBe(false);
    if (["feeding", "feeding-idle"].includes(sample.behavior)) expect(sample.gap).toBeCloseTo(6, 1);
    if (sample.behavior === "feeding") {
      expect(sample.image).toBe(`extras/dekopon/${sample.count === 0 ? "feeding" : `feeding-size-${sample.count}`}/feeding-strip.png`);
    }
  }
  for (const count of [1, 2, 3, 4]) {
    const before = samples.find((sample) => sample.behavior === "feeding-idle" && sample.count === count)!;
    const feeding = samples.filter((sample) => sample.behavior === "feeding" && sample.count === count);
    expect([...new Set(feeding.map((sample) => sample.frame))]).toEqual(["0:0", "0:1", "0:2", "0:3", "0:4", "0:5"]);
    // Paws and fruit move mid-feed: within 2 source pixels (1.3 CSS px) of the starting stage.
    expect(Math.min(...feeding.map((sample) => sample.width))).toBeGreaterThanOrEqual(before.width - 2 * 0.65);
    const after = samples.find((sample) => sample.behavior === "feeding-idle" && sample.count === count + 1)!;
    expect(feeding.at(-1)!.width).toBeLessThanOrEqual(after.width);
  }
});

test("a failed later-size strip prevents scene startup and recovers after retry", async ({ page }) => {
  await page.route("**/feeding-size-3/feeding-strip.png*", (route) => route.request().resourceType() === "image" ? route.abort() : route.continue());
  await page.goto(`${BASE_URL}/lucia-demo.html`);
  await expect(page.getByRole("alert")).toContainText("artwork could not be loaded");
  await expect(page.getByTestId("lucia-scene")).toHaveCount(0);
  await page.unroute("**/feeding-size-3/feeding-strip.png*");
  await page.getByRole("button", { name: "Hide Lucia", exact: true }).click();
  await page.getByRole("button", { name: "Show Lucia", exact: true }).click();
  await expect(page.getByTestId("lucia-scene")).toBeVisible();
});

test("inference interrupts a later-size feed without awarding its fruit", async ({ page }) => {
  await page.goto(`${BASE_URL}/lucia-demo.html`);
  await page.getByRole("group", { name: "Lucia controls" }).getByRole("button", { name: "Feed", exact: true }).click();
  const scene = page.getByTestId("lucia-scene");
  const feed = page.getByRole("button", { name: "Feed a dekopon", exact: true });
  await feed.click();
  await expect(scene).toHaveAttribute("data-fed-count", "1");
  await feed.click();
  await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-image", /feeding-size-1/);
  await page.getByRole("button", { name: "Simulate inference", exact: true }).click();
  await expect(scene).toHaveAttribute("data-behavior", "reading-enter");
  await expect(scene).toHaveAttribute("data-fed-count", "1");
  await expect(scene).toHaveAttribute("data-behavior", "feeding-idle", { timeout: 10_000 });
  await expect(scene).toHaveAttribute("data-fed-count", "1");
  await expect(page.getByTestId("lucia-sprite")).toHaveAttribute("data-frame", "0:1");
});
